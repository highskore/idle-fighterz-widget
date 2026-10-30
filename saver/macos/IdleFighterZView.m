// The macOS screensaver: a ScreenSaverView showing the frames ifz_saver (saver/src/lib.rs) hands it, the running widget's
// or a demo run's, in a layer scaled up nearest-neighbour, with the power level under it.
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
    uint8_t power[33];
    bool live, fresh;
} IfzShot;

void *ifz_saver_new(void);
bool ifz_saver_poll(void *s, IfzShot *out);
void ifz_saver_free(void *s);

@interface IFZ_VIEW : ScreenSaverView
@end

@implementation IFZ_VIEW {
    void *_saver;
    CALayer *_planet;
    CATextLayer *_power;
    CGColorSpaceRef _space;
    uint32_t _w, _h;
    NSString *_shown;
    int _live;
}

- (instancetype)initWithFrame:(NSRect)frame isPreview:(BOOL)isPreview {
    self = [super initWithFrame:frame isPreview:isPreview];
    if (!self) return nil;
    [self setAnimationTimeInterval:1.0 / 30.0];
    self.wantsLayer = YES;
    CGColorRef bg = CGColorCreateSRGB(7 / 255.0, 7 / 255.0, 12 / 255.0, 1);
    self.layer.backgroundColor = bg;
    CGColorRelease(bg);
    _live = -1;
    _space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    _planet = [CALayer layer];
    _planet.magnificationFilter = kCAFilterNearest;
    _planet.minificationFilter = kCAFilterNearest;
    [self.layer addSublayer:_planet];
    _power = [CATextLayer layer];
    _power.alignmentMode = kCAAlignmentCenter;
    _power.font = (__bridge CFTypeRef)[NSFont systemFontOfSize:12 weight:NSFontWeightSemibold];
    CGColorRef fg = CGColorCreateSRGB(0.85, 0.85, 0.88, 1);
    _power.foregroundColor = fg;
    CGColorRelease(fg);
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
    _planet.contents = nil;
    _power.string = nil;
    [CATransaction commit];
    _w = _h = 0;
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

- (void)animateOneFrame {
    if (!_saver) return;
    IfzShot s;
    if (!ifz_saver_poll(_saver, &s)) return;
    if (s.live != _live) {
        _live = s.live;
        NSLog(@"Idle FighterZ screensaver: %@", s.live ? @"showing the widget's run" : @"playing a demo run");
    }
    if (!s.fresh || s.w == 0 || s.h == 0) return;
    NSData *px = [NSData dataWithBytes:s.rgba length:(size_t)s.w * s.h * 4];
    CGDataProviderRef dp = CGDataProviderCreateWithCFData((__bridge CFDataRef)px);
    CGImageRef img = CGImageCreate(s.w, s.h, 8, 32, s.w * 4, _space, kCGBitmapByteOrderDefault | kCGImageAlphaLast, dp, NULL, false, kCGRenderingIntentDefault);
    CGDataProviderRelease(dp);
    if (!img) return;
    NSString *power = s.power[0] ? [NSString stringWithFormat:@"power %s", (const char *)s.power] : nil;
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    _planet.contents = (__bridge id)img;
    if (s.w != _w || s.h != _h) {
        _w = s.w;
        _h = s.h;
        [self place];
    }
    if (power != _shown && ![power isEqualToString:_shown]) {
        _shown = power;
        _power.string = power;
    }
    [CATransaction commit];
    CGImageRelease(img);
}

- (void)setFrameSize:(NSSize)size {
    [super setFrameSize:size];
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    [self place];
    [CATransaction commit];
}

// the frame centred a little above the middle at the biggest whole scale (in device pixels) that leaves a margin, or
// shrunk to fit a small preview; the power level under it
- (void)place {
    if (_w == 0) return;
    NSRect b = self.bounds;
    CGFloat ds = self.window.backingScaleFactor ?: (self.layer.contentsScale ?: 1);
    CGFloat fit = MIN(b.size.width * ds * 0.8 / _w, b.size.height * ds * 0.72 / _h);
    CGFloat k = fit >= 1 ? floor(fit) : fit;
    CGFloat w = _w * k / ds, h = _h * k / ds;
    CGFloat x = round((b.size.width - w) / 2 * ds) / ds;
    CGFloat y = round(((b.size.height - h) / 2 + b.size.height / 20) * ds) / ds;
    _planet.frame = CGRectMake(x, y, w, h);
    CGFloat pt = MAX(12, MIN(40, k * 9 / ds));
    BOOL text = !self.isPreview || b.size.height >= 200;
    _power.hidden = !text;
    _power.contentsScale = ds;
    _power.fontSize = pt;
    _power.frame = CGRectMake(0, y - pt * 2.2, b.size.width, pt * 1.5);
}

- (BOOL)hasConfigureSheet {
    return NO;
}

- (NSWindow *)configureSheet {
    return nil;
}

@end
