// Notifications through UNUserNotificationCenter: macOS asks the user once, and the answer can be read back for
// the settings page. It only works from inside a .app bundle (`tauri dev` runs a bare binary), so outside one it
// reports "unavailable" and posts nothing.
use block2::RcBlock;
use objc2::{define_class, msg_send, rc::Retained, runtime::ProtocolObject, AllocAnyThread};
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};
use std::{
    ptr::NonNull,
    sync::{atomic::AtomicU64, atomic::Ordering, mpsc, OnceLock},
    time::Duration,
};
use tauri::{AppHandle, Emitter};

static APP: OnceLock<AppHandle> = OnceLock::new();

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "PowerPlanetNotes"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        // the app counts as frontmost after a click on the widget; show the banner anyway
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _c: &UNUserNotificationCenter, _n: &UNNotification, done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
            done.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List | UNNotificationPresentationOptions::Sound,));
        }

        // a click on a notification brings the widget back
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _c: &UNUserNotificationCenter, _r: &UNNotificationResponse, done: &block2::DynBlock<dyn Fn()>) {
            if let Some(app) = APP.get() {
                let a = app.clone();
                let _ = app.run_on_main_thread(move || crate::show_widget(&a, true));
            }
            done.call(());
        }
    }
);

fn center() -> Option<Retained<UNUserNotificationCenter>> {
    NSBundle::mainBundle().bundlePath().to_string().ends_with(".app").then(UNUserNotificationCenter::currentNotificationCenter)
}

pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let Some(c) = center() else { return };
    let d: Retained<Delegate> = unsafe { msg_send![super(Delegate::alloc().set_ivars(())), init] };
    c.setDelegate(Some(ProtocolObject::from_ref(&*d)));
    std::mem::forget(d); // the centre holds its delegate weakly; this one lives as long as the app
}

fn name(s: UNAuthorizationStatus) -> &'static str {
    match s {
        UNAuthorizationStatus::NotDetermined => "ask",
        UNAuthorizationStatus::Denied => "denied",
        _ => "granted",
    }
}

fn status(c: &UNUserNotificationCenter, then: impl Fn(UNAuthorizationStatus) + 'static) {
    c.getNotificationSettingsWithCompletionHandler(&RcBlock::new(move |s: NonNull<UNNotificationSettings>| then(unsafe { s.as_ref() }.authorizationStatus())));
}

/// "granted", "denied", "ask" (not asked yet) or "unavailable" (not running from the .app)
pub fn permission() -> &'static str {
    let Some(c) = center() else { return "unavailable" };
    let (tx, rx) = mpsc::channel();
    status(&c, move |s| {
        let _ = tx.send(name(s));
    });
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or("ask")
}

/// shows the system prompt if it hasn't been answered yet, tells both pages the result, and runs `then` if allowed
pub fn request(then: impl Fn() + 'static) {
    let Some(c) = center() else { return };
    let opts = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound;
    c.requestAuthorizationWithOptions_completionHandler(
        opts,
        &RcBlock::new(move |ok: objc2::runtime::Bool, _e: *mut NSError| {
            // read the answer back without blocking: this runs on the centre's own queue
            if let (Some(app), Some(c)) = (APP.get(), center()) {
                status(&c, move |s| {
                    let _ = app.emit("permission", name(s));
                });
            }
            if ok.as_bool() {
                then();
            }
        }),
    );
}

pub fn post(title: String, body: String) {
    static N: AtomicU64 = AtomicU64::new(0);
    let Some(c) = center() else { return };
    let send = move || {
        let Some(c) = center() else { return };
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&title));
        content.setBody(&NSString::from_str(&body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let id = NSString::from_str(&format!("pp-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let req = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        c.addNotificationRequest_withCompletionHandler(&req, None);
    };
    // notifications are on by default, so the first one asks for permission itself if settings never did
    status(&c, move |s| match s {
        UNAuthorizationStatus::NotDetermined => request(send.clone()),
        UNAuthorizationStatus::Denied => {}
        _ => send(),
    });
}
