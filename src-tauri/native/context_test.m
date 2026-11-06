#import "visible_text_walk.h"
#import "capture_window_selection.h"
#include <assert.h>

static NSDictionary *node(NSString *role, NSString *value, NSArray *children, BOOL visible) {
    return @{@"AXRole":role, @"AXValue":value, @"children":children, @"visible":@(visible)};
}
static NSString *collect(id root, NSTimeInterval deadline) {
    return buddyVisibleText(root, deadline,
        ^NSString *(id n, NSString *key) { return n[key] ?: @""; },
        ^NSArray *(id n) { return n[@"children"]; },
        ^BOOL(id n) { return [n[@"visible"] boolValue]; });
}
int main(void) { @autoreleasepool {
    NSTimeInterval deadline = NSDate.timeIntervalSinceReferenceDate + 10;
    id message = node(@"AXStaticText", @"Prepare the launch checklist today", @[], YES);
    id deep = message;
    for (int i = 0; i < 13; i++) deep = node(@"AXGroup", @"", @[deep], YES);
    id root = node(@"AXWindow", @"", @[deep], YES);
    assert([collect(root, deadline) containsString:@"launch checklist"]);

    id editable = node(@"AXTextArea", @"draft", @[message], YES);
    id secure = @{@"AXRole":@"AXGroup", @"AXSubrole":@"AXSecureTextField", @"children":@[message]};
    id hidden = node(@"AXStaticText", @"Hidden message", @[], NO);
    id hiddenContainer = @{@"AXRole":@"AXGroup", @"AXHidden":@"true", @"children":@[message]};
    assert(collect(node(@"AXWindow", @"", @[editable, secure, hidden, hiddenContainer], YES), deadline).length == 0);
    id document = node(@"AXWebArea", @"", @[message], YES);
    id chrome = node(@"AXStaticText", @"Browser toolbar", @[], YES);
    NSString *content = collect(node(@"AXWindow", @"", @[chrome, document], YES), deadline);
    assert([content containsString:@"launch checklist"] && ![content containsString:@"toolbar"]);
    // No browser/document role is required for a native application's text.
    assert([collect(root, deadline) containsString:@"Prepare"]);
    // A web app's sidebar (a navigation landmark listing other conversations) and its
    // toolbar are chrome; the main landmark is the content, even when it comes last.
    id sidebar = @{@"AXRole":@"AXGroup", @"AXSubrole":@"AXLandmarkNavigation", @"visible":@YES,
        @"children":@[node(@"AXStaticText", @"Recents", @[], YES), node(@"AXStaticText", @"Brainstorm startup ideas", @[], YES)]};
    id toolbar = node(@"AXToolbar", @"", @[node(@"AXStaticText", @"Share", @[], YES)], YES);
    id banner = @{@"AXRole":@"AXGroup", @"AXSubrole":@"AXLandmarkBanner", @"visible":@YES,
        @"children":@[node(@"AXStaticText", @"Claude", @[], YES)]};
    id aside = @{@"AXRole":@"AXGroup", @"AXSubrole":@"AXLandmarkComplementary", @"visible":@YES,
        @"children":@[node(@"AXStaticText", @"Artifact panel", @[], YES)]};
    id mainArea = @{@"AXRole":@"AXGroup", @"AXSubrole":@"AXLandmarkMain", @"visible":@YES,
        @"children":@[node(@"AXStaticText", @"Right-to-work checks: the four-week plan", @[], YES)]};
    id webApp = node(@"AXWindow", @"", @[toolbar, node(@"AXWebArea", @"", @[banner, sidebar, aside, mainArea], YES)], YES);
    NSString *app = collect(webApp, deadline);
    assert([app containsString:@"Right-to-work"] && ![app containsString:@"Brainstorm"]
        && ![app containsString:@"Recents"] && ![app containsString:@"Share"] && ![app containsString:@"Artifact panel"]);
    // Without a main landmark the whole web area remains content, minus navigation chrome.
    id plainApp = node(@"AXWindow", @"", @[node(@"AXWebArea", @"", @[sidebar, aside], YES)], YES);
    NSString *plain = collect(plainApp, deadline);
    assert([plain containsString:@"Artifact panel"] && ![plain containsString:@"Brainstorm"]);
    NSMutableArray *wide = [NSMutableArray array];
    for (int i = 0; i < 40; i++) [wide addObject:node(@"AXButton", @"", @[], YES)];
    [wide addObject:document];
    assert([collect(node(@"AXWindow", @"", wide, YES), deadline) containsString:@"checklist"]);
    assert(collect(root, NSDate.timeIntervalSinceReferenceDate - 1).length == 0);
    NSString *longText = [@"x" stringByPaddingToLength:10000 withString:@"x" startingAtIndex:0];
    assert(collect(node(@"AXStaticText", longText, @[], YES), deadline).length == 3000);
    id tooDeep = message;
    for (int i = 0; i < 34; i++) tooDeep = node(@"AXGroup", @"", @[tooDeep], YES);
    assert(collect(tooDeep, deadline).length == 0);

    CGRect frame = CGRectMake(0, 33, 1470, 859);
    id window = @{@"pid":@42, @"frame":[NSValue valueWithRect:frame], @"title":@"Different capture title"};
    pid_t (^owner)(id) = ^pid_t(id w) { return [w[@"pid"] intValue]; };
    CGRect (^bounds)(id) = ^CGRect(id w) { return [w[@"frame"] rectValue]; };
    assert(buddyUniqueCaptureWindow(@[window], 42, frame, owner, bounds) == window);
    assert(!buddyUniqueCaptureWindow(@[window], 43, frame, owner, bounds));
    assert(!buddyUniqueCaptureWindow(@[window, window], 42, frame, owner, bounds));
    assert(!buddyUniqueCaptureWindow(@[window], 42, CGRectOffset(frame, 4, 0), owner, bounds));
    assert(!buddyUniqueCaptureWindow(@[window], 42, CGRectNull, owner, bounds));
    puts("Native context regressions passed: deep/wide trees, visible content, editable/secure exclusions, budgets, unique window capture.");
} return 0; }
