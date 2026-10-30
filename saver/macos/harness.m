// A test host for the .saver: loads it as legacyScreenSaver would, runs it in an off-screen window, writes a few
// frames as PNGs and reports the CPU it used while running and after stopAnimation. Built and run by
// saver/macos/try.sh; with SANDBOX=1 that signs it with legacyScreenSaver's sandbox entitlements.
//
//   harness <bundle.saver> <seconds> <shots dir> [width height]
#import <Cocoa/Cocoa.h>
#import <ScreenSaver/ScreenSaver.h>
#include <sys/resource.h>

static double cpu(void) {
    struct rusage u;
    getrusage(RUSAGE_SELF, &u);
    return u.ru_utime.tv_sec + u.ru_utime.tv_usec / 1e6 + u.ru_stime.tv_sec + u.ru_stime.tv_usec / 1e6;
}

static void spin(double s) {
    [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:s]];
}

static void shot(NSView *v, NSString *path) {
    CGFloat ds = 2;
    size_t w = v.bounds.size.width * ds, h = v.bounds.size.height * ds;
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef c = CGBitmapContextCreate(NULL, w, h, 8, 0, cs, kCGImageAlphaPremultipliedLast);
    CGContextSetInterpolationQuality(c, kCGInterpolationNone);
    CGContextScaleCTM(c, ds, ds);
    [v.layer renderInContext:c];
    CGImageRef img = CGBitmapContextCreateImage(c);
    NSBitmapImageRep *rep = [[NSBitmapImageRep alloc] initWithCGImage:img];
    [[rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}] writeToFile:path atomically:YES];
    CGImageRelease(img);
    CGContextRelease(c);
    CGColorSpaceRelease(cs);
}

int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc < 4) {
            fprintf(stderr, "usage: harness <bundle.saver> <seconds> <shots dir> [width height]\n");
            return 2;
        }
        NSString *path = @(argv[1]), *dir = @(argv[3]);
        double secs = atof(argv[2]);
        CGFloat w = argc > 5 ? atof(argv[4]) : 1280, h = argc > 5 ? atof(argv[5]) : 800;
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
        NSBundle *b = [NSBundle bundleWithPath:path];
        NSError *err = nil;
        if (![b loadAndReturnError:&err]) {
            fprintf(stderr, "load failed: %s\n", err.description.UTF8String);
            return 1;
        }
        Class cls = b.principalClass;
        NSRect r = NSMakeRect(-20000, -20000, w, h);
        NSWindow *win = [[NSWindow alloc] initWithContentRect:r styleMask:NSWindowStyleMaskBorderless backing:NSBackingStoreBuffered defer:NO];
        ScreenSaverView *v = [[cls alloc] initWithFrame:NSMakeRect(0, 0, w, h) isPreview:NO];
        win.contentView = v;
        [win orderBack:nil];
        [[NSFileManager defaultManager] createDirectoryAtPath:dir withIntermediateDirectories:YES attributes:nil error:nil];
        double c0 = cpu();
        [v startAnimation];
        int n = 0;
        for (double t = 0; t < secs; t += 1) {
            spin(1);
            if ((int)t % 5 == 4) shot(v, [dir stringByAppendingPathComponent:[NSString stringWithFormat:@"frame-%02d.png", n++]]);
        }
        double c1 = cpu();
        [v stopAnimation];
        spin(4);
        double c2 = cpu();
        printf("running: %.1f%% of a core over %.0f s\n", (c1 - c0) / secs * 100, secs);
        printf("after stopAnimation: %.2f%% of a core over 4 s\n", (c2 - c1) / 4 * 100);
        fflush(stdout);
    }
    return 0;
}
