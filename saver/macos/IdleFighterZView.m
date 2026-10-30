// The macOS screensaver: a ScreenSaverView showing the pictures ifz_saver (saver/src/lib.rs) hands it, the running
// widget's planet or a demo run's over the site's sky and cloud sea, in a layer scaled up nearest-neighbour, with the
// power level under the planet in plain text.
//
// Since Sonoma every screensaver runs in the sandboxed legacyScreenSaver host, which has two known faults: it can
// outlive the screensaver (and keep calling animateOneFrame unseen), and stopAnimation may never come. So the view
// stops itself on the engine's willstop notification and then ends the host, and the Rust side lets go of the widget
// (which then stops drawing for it) as soon as nothing has polled it for two seconds.
#import <QuartzCore/QuartzCore.h>
#import <ScreenSaver/ScreenSaver.h>
#include <stdbool.h>
#include <stdint.h>

#ifndef IFZ_VIEW
#define IFZ_VIEW IdleFighterZView
#endif

typedef struct {
    const uint8_t *rgba;
    uint32_t w, h;
    double scale;
    uint8_t power[33];
    double power_y, power_px;
    uint32_t muted, ink;
    bool live, fresh;
} IfzShot;

void *ifz_saver_new(void);
bool ifz_saver_poll(void *s, double w, double h, IfzShot *out);
void ifz_saver_free(void *s);

@interface IFZ_VIEW : ScreenSaverView
@end

@implementation IFZ_VIEW {
    void *_saver;
    CALayer *_scene;
    CATextLayer *_power;
    CGColorSpaceRef _space;
    NSString *_shown;
    uint32_t _ink;
    CGFloat _pt;
    int _live;
}

- (instancetype)initWithFrame:(NSRect)frame isPreview:(BOOL)isPreview {
    self = [super initWithFrame:frame isPreview:isPreview];
    if (!self) return nil;
    [self setAnimationTimeInterval:1.0 / 30.0];
    self.wantsLayer = YES;
    CGColorRef bg = CGColorCreateSRGB(0xfc / 255.0, 0xfc / 255.0, 0xfb / 255.0, 1);
    self.layer.backgroundColor = bg;
    CGColorRelease(bg);
    self.layer.masksToBounds = YES;
    _live = -1;
    _space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    _scene = [CALayer layer];
    _scene.magnificationFilter = kCAFilterNearest;
    _scene.minificationFilter = kCAFilterNearest;
    _scene.opaque = YES;
    [self.layer addSublayer:_scene];
    _power = [CATextLayer layer];
    _power.alignmentMode = kCAAlignmentCenter;
    [self.layer addSublayer:_power];
    if (!isPreview) {
        [[NSDistributedNotificationCenter defaultCenter] addObserver:self selector:@selector(willStop:) name:@"com.apple.screensaver.willstop" object:nil];
    }
    return self;
}

- (void)dealloc {
    [[NSDistributedNotificationCenter defaultCenter] removeObserver:self];
    [self teardown];
    CGColorSpaceRelease(_space);
}

- (void)startAnimation {
    [super startAnimation];
    if (!_saver) _saver = ifz_saver_new();
}

- (void)stopAnimation {
    [super stopAnimation];
    [self teardown];
}

// closes the link to the widget and drops the demo: nothing of ours runs after this
- (void)teardown {
    if (_saver) {
        ifz_saver_free(_saver);
        _saver = NULL;
    }
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    _scene.contents = nil;
    _power.string = nil;
    [CATransaction commit];
    _shown = nil;
}

// the engine is dismissing the screensaver: stop now, and end the host, which on Sonoma and later otherwise lingers
// (and can keep animating unseen) after the screensaver is gone
- (void)willStop:(NSNotification *)n {
    [self stopAnimation];
    if (!self.isPreview) exit(0);
}

- (void)viewDidMoveToWindow {
    [super viewDidMoveToWindow];
    if (!self.window) [self teardown];
}

static NSColor *hex(uint32_t c) {
    return [NSColor colorWithSRGBRed:((c >> 16) & 255) / 255.0 green:((c >> 8) & 255) / 255.0 blue:(c & 255) / 255.0 alpha:1];
}

- (void)animateOneFrame {
    if (!_saver) return;
    NSRect b = self.bounds;
    CGFloat ds = self.window.backingScaleFactor ?: (self.layer.contentsScale ?: 1);
    IfzShot s;
    if (!ifz_saver_poll(_saver, b.size.width * ds, b.size.height * ds, &s)) return;
    if (s.live != _live) {
        _live = s.live;
        NSLog(@"Idle FighterZ screensaver: %@", s.live ? @"showing the widget's run" : @"playing a demo run");
    }
    if (!s.fresh || s.w == 0 || s.h == 0) return;
    NSData *px = [NSData dataWithBytes:s.rgba length:(size_t)s.w * s.h * 4];
    CGDataProviderRef dp = CGDataProviderCreateWithCFData((__bridge CFDataRef)px);
    CGImageRef img = CGImageCreate(s.w, s.h, 8, 32, s.w * 4, _space, kCGBitmapByteOrderDefault | kCGImageAlphaNoneSkipLast, dp, NULL, false, kCGRenderingIntentDefault);
    CGDataProviderRelease(dp);
    if (!img) return;
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    // from the top left: past the right and bottom edges is cut off
    CGFloat w = s.w * s.scale / ds, h = s.h * s.scale / ds;
    _scene.frame = CGRectMake(0, b.size.height - h, w, h);
    _scene.contents = (__bridge id)img;
    NSString *power = s.power[0] && s.power_px > 0 ? @((const char *)s.power) : nil;
    CGFloat pt = s.power_px / ds;
    if ((power != _shown && ![power isEqualToString:_shown]) || s.ink != _ink || pt != _pt) {
        _shown = power;
        _ink = s.ink;
        _pt = pt;
        _power.hidden = !power;
        if (power) {
            NSFont *f = [NSFont monospacedDigitSystemFontOfSize:pt weight:NSFontWeightRegular];
            NSMutableAttributedString *t = [[NSMutableAttributedString alloc] initWithString:@"power " attributes:@{NSFontAttributeName : f, NSForegroundColorAttributeName : hex(s.muted)}];
            [t appendAttributedString:[[NSAttributedString alloc] initWithString:power attributes:@{NSFontAttributeName : f, NSForegroundColorAttributeName : hex(s.ink)}]];
            _power.string = t;
        }
    }
    _power.contentsScale = ds;
    _power.frame = CGRectMake(0, round(b.size.height - s.power_y / ds - pt * 0.65), b.size.width, ceil(pt * 1.4));
    [CATransaction commit];
    CGImageRelease(img);
}

- (BOOL)hasConfigureSheet {
    return NO;
}

- (NSWindow *)configureSheet {
    return nil;
}

@end
