#import <Foundation/Foundation.h>

// A bounded traversal shared with the native regression harness. Attribute and
// geometry access stay in the platform adapter; no window text is logged here.
//
// Reading order is preserved, but application chrome is not content: navigation,
// banner, footer and search landmarks, toolbars and menus are skipped so the
// budget reaches the document itself. A web area or document resets the output
// once (browser and window chrome before it is dropped); a main landmark inside
// it resets it once more, so a web app's sidebar never displaces its main view.
static NSString *buddyVisibleText(id root, NSTimeInterval deadline,
    NSString *(^attribute)(id, NSString *), NSArray *(^children)(id), BOOL (^visible)(id)) {
    NSMutableString *output = [NSMutableString string];
    NSMutableArray *stack = [NSMutableArray arrayWithObject:@[root, @0, @NO, @NO]];
    NSUInteger visited = 0;
    BOOL foundContent = NO, foundMain = NO;
    while (stack.count && visited++ < 1024 && NSDate.timeIntervalSinceReferenceDate <= deadline) {
        NSArray *entry = stack.lastObject;
        [stack removeLastObject];
        id node = entry[0];
        NSUInteger depth = [entry[1] unsignedIntegerValue];
        NSString *role = attribute(node, @"AXRole");
        NSString *subrole = attribute(node, @"AXSubrole");
        if ([attribute(node, @"AXHidden") isEqualToString:@"true"]
            || [subrole isEqualToString:@"AXSecureTextField"]
            || [@[@"AXTextField", @"AXTextArea", @"AXComboBox"] containsObject:role]) continue;
        if ([@[@"AXLandmarkNavigation", @"AXLandmarkBanner", @"AXLandmarkContentInfo", @"AXLandmarkSearch"] containsObject:subrole]
            || [@[@"AXToolbar", @"AXMenuBar", @"AXMenu"] containsObject:role]) continue;
        BOOL content = [entry[2] boolValue] || [@[@"AXWebArea", @"AXDocument"] containsObject:role];
        BOOL main = [entry[3] boolValue] || [subrole isEqualToString:@"AXLandmarkMain"];
        if (content && !foundContent) { [output setString:@""]; foundContent = YES; }
        if (main && !foundMain) { [output setString:@""]; foundMain = YES; }
        BOOL wanted = (!foundContent || content) && (!foundMain || main);
        if (wanted && [role isEqualToString:@"AXStaticText"] && visible(node) && output.length < 3000) {
            NSString *value = attribute(node, @"AXValue");
            NSUInteger length = MIN(value.length, 3000 - output.length);
            if (length) {
                [output appendString:[value substringToIndex:length]];
                if (output.length < 3000) [output appendString:@"\n"];
            }
        }
        if (depth >= 32 || (content && output.length >= 3000)) continue;
        NSArray *nodes = children(node);
        // Preserve reading order, and bound memory even for pathological trees.
        NSUInteger count = MIN(nodes.count, MIN((NSUInteger)64, 1024 - stack.count));
        for (NSUInteger index = count; index > 0; --index)
            [stack addObject:@[nodes[index - 1], @(depth + 1), @(content), @(main)]];
    }
    return output;
}
