#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <Security/Security.h>
#import <Carbon/Carbon.h>
#import <Speech/Speech.h>
#import <CoreAudio/CoreAudio.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import "visible_text_walk.h"
#include <sys/stat.h>
#include <libproc.h>

// Every exported string is owned by Rust after the call and freed with buddy_free.
static char *json(id value) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:value options:NSJSONWritingFragmentsAllowed error:nil];
    return strdup(data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String : "null");
}
static char *failure(NSString *message) { return json(@{@"error": message}); }
void buddy_free(char *value) { free(value); }
// Separate local evidence key; the immutable provider helper is unchanged.
char *buddy_review_key(const char *secret) {
    @autoreleasepool {
        NSMutableDictionary *query = [@{(__bridge id)kSecClass:(__bridge id)kSecClassGenericPassword,
            (__bridge id)kSecAttrService:@"com.artmarketvm.desktopbuddy.activity-review",
            (__bridge id)kSecAttrAccount:@"evidence-v1"} mutableCopy];
        if (secret) {
            NSData *data = [[NSString stringWithUTF8String:secret] dataUsingEncoding:NSUTF8StringEncoding];
            OSStatus status = SecItemUpdate((__bridge CFDictionaryRef)query,(__bridge CFDictionaryRef)@{(__bridge id)kSecValueData:data});
            if (status == errSecItemNotFound) {
                query[(__bridge id)kSecValueData] = data;
                query[(__bridge id)kSecAttrAccessible] = (__bridge id)kSecAttrAccessibleWhenUnlockedThisDeviceOnly;
                status = SecItemAdd((__bridge CFDictionaryRef)query,NULL);
            }
            if (status != errSecSuccess) return failure(@"Protected activity storage is unavailable");
            return json([NSString stringWithUTF8String:secret]);
        }
        query[(__bridge id)kSecReturnData] = @YES;
        CFTypeRef result = NULL;
        OSStatus status = SecItemCopyMatching((__bridge CFDictionaryRef)query,&result);
        if (status == errSecItemNotFound) return json(NSNull.null);
        if (status != errSecSuccess) return failure(@"Protected activity storage is unavailable");
        NSData *data = CFBridgingRelease(result);
        NSString *value = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
        return value ? json(value) : failure(@"Protected activity storage is unavailable");
    }
}
static id attribute(AXUIElementRef element, CFStringRef name) {
    CFTypeRef value = NULL;
    if (!element || AXUIElementCopyAttributeValue(element, name, &value) != kAXErrorSuccess) return nil;
    return CFBridgingRelease(value);
}
static NSString *text(id value, NSUInteger limit) {
    if (![value isKindOfClass:NSString.class]) return @"";
    NSString *string = value;
    return [string substringToIndex:MIN(limit, string.length)];
}
static AXUIElementRef focusedWindow(pid_t pid) {
    AXUIElementRef application = AXUIElementCreateApplication(pid);
    AXUIElementSetMessagingTimeout(application, 0.08);
    CFTypeRef window = NULL;
    AXUIElementCopyAttributeValue(application, kAXFocusedWindowAttribute, &window);
    CFRelease(application);
    if (window && CFGetTypeID(window) != AXUIElementGetTypeID()) {
        CFRelease(window); return NULL;
    }
    if (window) AXUIElementSetMessagingTimeout((AXUIElementRef)window, 0.08);
    return (AXUIElementRef)window;
}
static uint64_t windowID(pid_t pid, AXUIElementRef window) {
    return ((uint64_t)(uint32_t)pid << 32) | (window ? (uint32_t)CFHash(window) : 0);
}
static NSString *canonicalProcess(NSString *executable, NSString *bundle) {
    NSDictionary *aliases = @{@"com.microsoft.VSCode": @"code", @"com.apple.dt.Xcode": @"xcode", @"com.adobe.Photoshop": @"adobe photoshop", @"com.adobe.illustrator": @"adobe illustrator", @"com.microsoft.teams2": @"microsoft teams", @"com.microsoft.teams": @"microsoft teams"};
    return aliases[bundle ?: @""] ?: executable ?: @"Unknown";
}
static NSString *processName(NSRunningApplication *application) {
    return canonicalProcess(application.executableURL.lastPathComponent ?: application.localizedName, application.bundleIdentifier);
}
// Public Quartz session API. The lock marker is supplied by WindowServer; absent
// markers mean unlocked, but missing/non-console session data always fails closed.
bool buddy_session_active(void) {
    NSDictionary *session = CFBridgingRelease(CGSessionCopyCurrentDictionary());
    return session && [session[(__bridge NSString *)kCGSessionOnConsoleKey] boolValue]
        && [session[(__bridge NSString *)kCGSessionLoginDoneKey] boolValue]
        && ![session[@"CGSSessionScreenIsLocked"] boolValue];
}
bool buddy_accessibility(bool prompt) {
    return AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)@{(__bridge NSString *)kAXTrustedCheckOptionPrompt: @(prompt)});
}
double buddy_idle(void) {
    return CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateCombinedSessionState, kCGAnyInputEventType);
}
// Output IO from the foreground app or a helper inside its own bundle only.
// Global device activity includes unrelated music; IO never proves attention or completion.
static BOOL processOutput(AudioObjectID object) {
    UInt32 running = 0, size = sizeof(running);
    AudioObjectPropertyAddress address = {kAudioProcessPropertyIsRunningOutput, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    return AudioObjectGetPropertyData(object, &address, 0, NULL, &size, &running) == noErr && running != 0;
}
static BOOL audioPlaying(NSRunningApplication *application) {
    pid_t pid = application.processIdentifier;
    AudioObjectID object = kAudioObjectUnknown;
    UInt32 size = sizeof(object);
    AudioObjectPropertyAddress address = {kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, sizeof(pid), &pid, &size, &object) == noErr && object != kAudioObjectUnknown && processOutput(object)) return YES;
    // Chromium and other apps put audio in a child process. Require the actual bundle path,
    // not a matching name or a bundle-ID prefix belonging to an unrelated application.
    NSString *bundle = application.bundleURL.URLByResolvingSymlinksInPath.path;
    if (![bundle.pathExtension isEqualToString:@"app"]) return NO;
    NSString *prefix = [bundle stringByAppendingString:@"/"];
    address.mSelector = kAudioHardwarePropertyProcessObjectList;
    if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &address, 0, NULL, &size) != noErr || size > 512 * sizeof(AudioObjectID)) return NO;
    AudioObjectID processes[512];
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, processes) != noErr) return NO;
    for (UInt32 i = 0; i < size / sizeof(AudioObjectID); i++) {
        if (!processOutput(processes[i])) continue;
        pid_t child = 0; UInt32 bytes = sizeof(child);
        AudioObjectPropertyAddress property = {kAudioProcessPropertyPID, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
        if (AudioObjectGetPropertyData(processes[i], &property, 0, NULL, &bytes, &child) != noErr) continue;
        char executable[PROC_PIDPATHINFO_MAXSIZE] = {0};
        if (proc_pidpath(child, executable, sizeof(executable)) <= 0) continue;
        NSString *path = [[NSString stringWithUTF8String:executable] stringByResolvingSymlinksInPath];
        if ([path hasPrefix:prefix]) return YES;
    }
    return NO;
}
// Browser metadata reads browser chrome only. Never traverse web document nodes.
// Safari and Chromium browsers publish the front page address on the window itself.
// Reading it costs one attribute, so wide tab bars cannot exhaust the traversal budget.
static NSString *documentAddress(AXUIElementRef window) {
    NSString *document = text(attribute(window, kAXDocumentAttribute), 4096);
    return ([document hasPrefix:@"http://"] || [document hasPrefix:@"https://"]) ? document : nil;
}
static NSString *addressBar(AXUIElementRef root, NSUInteger depth, NSUInteger *visited, NSTimeInterval deadline) {
    if (!root || depth > 7 || ++*visited > 120 || NSDate.timeIntervalSinceReferenceDate > deadline) return nil;
    NSString *role = attribute(root, kAXRoleAttribute);
    NSString *subrole = attribute(root, kAXSubroleAttribute);
    // Tab strips hold no address control and can be long; skipping them keeps the budget for the toolbar.
    if ([role isEqualToString:@"AXWebArea"] || [role isEqualToString:@"AXTabGroup"] || [subrole isEqualToString:@"AXSecureTextField"]) return nil;
    if ([role isEqualToString:@"AXTextField"] || [role isEqualToString:@"AXComboBox"]) {
        NSString *description = [text(attribute(root, kAXDescriptionAttribute), 200) lowercaseString];
        NSString *identifier = [text(attribute(root, kAXIdentifierAttribute), 200) lowercaseString];
        if ([description containsString:@"address"] || [description containsString:@"url"] || [description containsString:@"search or enter website"] || [identifier containsString:@"urlbar"] || [identifier containsString:@"address"]) {
            return text(attribute(root, kAXValueAttribute), 4096);
        }
    }
    CFArrayRef children = NULL;
    if (AXUIElementCopyAttributeValues(root, kAXChildrenAttribute, 0, 25, &children) == kAXErrorSuccess) {
        for (id child in (__bridge NSArray *)children) {
            NSString *address = addressBar((__bridge AXUIElementRef)child, depth + 1, visited, deadline);
            if (address.length) { CFRelease(children); return address; }
        }
        CFRelease(children);
    }
    return nil;
}
// The document behind a window: an editor's file (AXDocument) or, for web apps and other
// Chromium-based applications, the web area's document URL without query or fragment,
// falling back to a web-area title that differs from the window title. Windows that share
// one title (chat apps, notes, editors) are told apart by it. The collector keeps only a
// hash; the value itself is never stored or disclosed.
static NSString *trimmedAddress(id value) {
    NSString *string = [value isKindOfClass:NSURL.class] ? [(NSURL *)value absoluteString] : ([value isKindOfClass:NSString.class] ? value : nil);
    if (!string.length) return nil;
    NSRange cut = [string rangeOfCharacterFromSet:[NSCharacterSet characterSetWithCharactersInString:@"?#"]];
    if (cut.location != NSNotFound) string = [string substringToIndex:cut.location];
    return string.length ? [string substringToIndex:MIN((NSUInteger)512, string.length)] : nil;
}
static NSString *documentIdentity(AXUIElementRef window, NSString *title, NSTimeInterval deadline) {
    NSString *document = trimmedAddress(attribute(window, kAXDocumentAttribute));
    if (document) return document;
    NSMutableArray *queue = [NSMutableArray arrayWithObject:(__bridge id)window];
    NSUInteger visited = 0, index = 0;
    while (index < queue.count && visited++ < 96 && NSDate.timeIntervalSinceReferenceDate <= deadline) {
        AXUIElementRef node = (__bridge AXUIElementRef)queue[index++];
        NSString *role = attribute(node, kAXRoleAttribute);
        if ([role isEqualToString:@"AXWebArea"]) {
            NSString *address = trimmedAddress(attribute(node, kAXURLAttribute)) ?: trimmedAddress(attribute(node, kAXDocumentAttribute));
            if (address) return address;
            NSString *heading = text(attribute(node, kAXTitleAttribute), 160);
            return (heading.length && ![heading isEqualToString:title]) ? heading : nil;
        }
        if ([@[@"AXMenuBar", @"AXToolbar", @"AXStaticText", @"AXButton", @"AXTextField", @"AXTextArea"] containsObject:role] || queue.count > 160) continue;
        CFArrayRef children = NULL;
        if (AXUIElementCopyAttributeValues(node, kAXChildrenAttribute, 0, 32, &children) == kAXErrorSuccess) {
            [queue addObjectsFromArray:(__bridge NSArray *)children];
            CFRelease(children);
        }
    }
    return nil;
}
char *buddy_foreground(bool metadata) {
    @autoreleasepool {
        if (!buddy_session_active()) return failure(@"Desktop session is inactive or locked");
        if (!buddy_accessibility(false)) return failure(@"Allow sumi in System Settings > Privacy & Security > Accessibility, then resume tracking.");
        NSRunningApplication *application = NSWorkspace.sharedWorkspace.frontmostApplication;
        if (!application) return failure(@"No foreground application is available");
        pid_t pid = application.processIdentifier;
        AXUIElementRef window = focusedWindow(pid);
        NSString *title = window ? text(attribute(window, kAXTitleAttribute), 160) : application.localizedName;
        NSString *process = processName(application);
        NSString *address = nil;
        if (metadata && [@[@"google chrome", @"microsoft edge", @"firefox", @"safari", @"brave browser", @"arc"] containsObject:process.lowercaseString]) {
            address = documentAddress(window);
            if (!address) {
                NSUInteger visited = 0;
                address = addressBar(window, 0, &visited, NSDate.timeIntervalSinceReferenceDate + 0.20);
            }
        }
        BOOL fullscreen = [attribute(window, CFSTR("AXFullScreen")) boolValue];
        NSString *document = window ? documentIdentity(window, title, NSDate.timeIntervalSinceReferenceDate + 0.08) : nil;
        uint64_t identity = windowID(pid, window);
        AXUIElementRef latest = focusedWindow(pid);
        BOOL changed = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier != pid || identity != windowID(pid, latest) || ![(latest ? text(attribute(latest, kAXTitleAttribute), 160) : application.localizedName) isEqualToString:title];
        if (window) CFRelease(window);
        if (latest) CFRelease(latest);
        if (changed) return failure(@"Foreground context changed during collection");
        return json(@{@"process": process, @"title": title ?: @"", @"window": @(identity), @"idle": @(buddy_idle()), @"fullscreen": @(fullscreen), @"media": @(audioPlaying(application)), @"address": address ?: NSNull.null, @"document": document ?: NSNull.null});
    }
}
// Keep the mascot on every Space and above full-screen apps so it never gets left behind.
void buddy_pin_window(void *handle) {
    if (!handle) return;
    NSWindow *window = (__bridge NSWindow *)handle;
    dispatch_async(dispatch_get_main_queue(), ^{
        window.collectionBehavior |= NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorStationary | NSWindowCollectionBehaviorIgnoresCycle;
        window.level = NSStatusWindowLevel;
        window.hidesOnDeactivate = NO;
    });
}
// Showing a reminder must preserve the user's current key window and application.
void buddy_show_without_focus(void *handle) {
    if (!handle) return;
    NSWindow *window = (__bridge NSWindow *)handle;
    dispatch_async(dispatch_get_main_queue(), ^{
        [window orderFrontRegardless];
    });
}
bool buddy_fullscreen(void) {
    @autoreleasepool {
        if (!buddy_accessibility(false)) return false;
        AXUIElementRef window = focusedWindow(NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier);
        BOOL fullscreen = [attribute(window, CFSTR("AXFullScreen")) boolValue];
        if (window) CFRelease(window);
        return fullscreen;
    }
}
// Selected text uses only AXSelectedText; automatic context skips editable and secure controls.
static CGRect elementFrame(AXUIElementRef element) {
    id position = attribute(element, kAXPositionAttribute), size = attribute(element, kAXSizeAttribute);
    CGPoint origin = CGPointZero; CGSize dimensions = CGSizeZero;
    if (!position || !size || CFGetTypeID((__bridge CFTypeRef)position) != AXValueGetTypeID() || CFGetTypeID((__bridge CFTypeRef)size) != AXValueGetTypeID()) return CGRectNull;
    if (!AXValueGetValue((__bridge AXValueRef)position, kAXValueCGPointType, &origin) || !AXValueGetValue((__bridge AXValueRef)size, kAXValueCGSizeType, &dimensions)) return CGRectNull;
    return (CGRect){origin, dimensions};
}
char *buddy_context(uint64_t identity, bool selected) {
    @autoreleasepool {
        if (!buddy_session_active()) return failure(@"Desktop session is inactive or locked");
        if (!buddy_accessibility(false)) return failure(@"Allow sumi in Accessibility settings, or paste the text into sumi.");
        pid_t pid = (pid_t)(identity >> 32);
        if (NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier != pid) return failure(@"Screen context changed");
        AXUIElementRef window = focusedWindow(pid);
        if (!window || windowID(pid, window) != identity) { if (window) CFRelease(window); return failure(@"Screen context changed"); }
        NSMutableString *output = [NSMutableString string];
        if (selected) {
            AXUIElementRef app = AXUIElementCreateApplication(pid);
            AXUIElementSetMessagingTimeout(app, 0.08);
            id focused = attribute(app, kAXFocusedUIElementAttribute);
            if (focused && CFGetTypeID((__bridge CFTypeRef)focused) == AXUIElementGetTypeID()) {
                AXUIElementRef element = (__bridge AXUIElementRef)focused;
                if (![attribute(element, kAXSubroleAttribute) isEqualToString:@"AXSecureTextField"]) [output appendString:text(attribute(element, kAXSelectedTextAttribute), 4001)];
            }
            CFRelease(app);
        } else {
            CGRect frame = elementFrame(window);
            // Chrome is skipped and content regions take priority, so the budget is spent
            // on the document; deep web-app trees need more than a browser toolbar did.
            [output appendString:buddyVisibleText((__bridge id)window,
                NSDate.timeIntervalSinceReferenceDate + 0.45,
                ^NSString *(id node, NSString *name) {
                    if ([name isEqualToString:@"AXHidden"])
                        return [attribute((__bridge AXUIElementRef)node, kAXHiddenAttribute) boolValue] ? @"true" : @"";
                    return text(attribute((__bridge AXUIElementRef)node, (__bridge CFStringRef)name), 3000);
                }, ^NSArray *(id node) {
                    CFArrayRef children = NULL;
                    if (AXUIElementCopyAttributeValues((__bridge AXUIElementRef)node, kAXChildrenAttribute, 0, 64, &children) != kAXErrorSuccess) return @[];
                    return CFBridgingRelease(children);
                }, ^BOOL(id node) {
                    CGRect bounds = elementFrame((__bridge AXUIElementRef)node);
                    return !CGRectIsNull(bounds) && !CGRectIsEmpty(bounds) && CGRectIntersectsRect(bounds, frame);
                })];
        }
        AXUIElementRef latest = focusedWindow(pid);
        BOOL changed = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier != pid || windowID(pid, latest) != identity;
        CFRelease(window); if (latest) CFRelease(latest);
        if (changed) return failure(@"Screen context changed");
        return json(output);
    }
}
char *buddy_apps(void) {
    @autoreleasepool {
        NSMutableArray *apps = [NSMutableArray array];
        NSArray *roots = @[@"/Applications", @"/System/Applications", [NSHomeDirectory() stringByAppendingPathComponent:@"Applications"]];
        for (NSString *root in roots) {
            NSDirectoryEnumerator *enumerator = [NSFileManager.defaultManager enumeratorAtURL:[NSURL fileURLWithPath:root] includingPropertiesForKeys:@[NSURLIsDirectoryKey, NSURLIsSymbolicLinkKey] options:NSDirectoryEnumerationSkipsHiddenFiles errorHandler:nil];
            NSUInteger visited = 0;
            for (NSURL *url in enumerator) {
                if (++visited > 5000) break;
                NSNumber *symlink = nil; [url getResourceValue:&symlink forKey:NSURLIsSymbolicLinkKey error:nil];
                if (symlink.boolValue) { [enumerator skipDescendants]; continue; }
                if ([url.pathExtension isEqualToString:@"app"]) {
                    [enumerator skipDescendants];
                    NSDictionary *info = [NSDictionary dictionaryWithContentsOfURL:[url URLByAppendingPathComponent:@"Contents/Info.plist"]];
                    NSString *process = canonicalProcess(info[@"CFBundleExecutable"], info[@"CFBundleIdentifier"]);
                    NSString *name = info[@"CFBundleDisplayName"] ?: info[@"CFBundleName"] ?: url.lastPathComponent.stringByDeletingPathExtension;
                    if ([process isKindOfClass:NSString.class] && process.length && process.length <= 100 && [name isKindOfClass:NSString.class]) [apps addObject:@{@"process_name": process, @"name": name}];
                } else if ([[url.path substringFromIndex:root.length] componentsSeparatedByString:@"/"].count > 3) [enumerator skipDescendants];
            }
        }
        return json(apps);
    }
}
char *buddy_keychain(const char *account, const char *secret, int operation) {
    @autoreleasepool {
        NSURL *helper = [NSBundle.mainBundle.bundleURL URLByAppendingPathComponent:@"Contents/Helpers/BuddyCredentialAccess"];
        SecStaticCodeRef code = NULL; SecRequirementRef requirement = NULL;
        OSStatus status = SecStaticCodeCreateWithPath((__bridge CFURLRef)helper, kSecCSDefaultFlags, &code);
        if (status == errSecSuccess) status = SecRequirementCreateWithString(CFSTR("identifier \"com.artmarketvm.desktopbuddy.credentials\" and certificate root = H\"9b85d29dad2e9328ec27631337e3e9313383ab20\""), kSecCSDefaultFlags, &requirement);
        if (status == errSecSuccess) status = SecStaticCodeCheckValidity(code, kSecCSStrictValidate, requirement);
        if (requirement) CFRelease(requirement);
        if (code) CFRelease(code);
        if (status != errSecSuccess) return failure(@"The signed credential helper is missing or invalid. Reinstall sumi from the official release.");
        NSArray *operations = @[@"read", @"write", @"remove", @"exists"];
        if (operation < 0 || operation > 3) return failure(@"Invalid credential operation");
        NSMutableDictionary *request = [@{@"account":@(account), @"operation":operations[operation]} mutableCopy];
        if (operation == 1 && secret) request[@"secret"] = @(secret);
        NSTask *task = [NSTask new]; task.executableURL = helper;
        NSPipe *input = NSPipe.pipe, *output = NSPipe.pipe;
        task.standardInput = input; task.standardOutput = output;
        task.standardError = NSFileHandle.fileHandleWithNullDevice;
        NSError *error = nil;
        if (![task launchAndReturnError:&error]) return failure(@"Could not start the signed credential helper");
        @try {
            [input.fileHandleForWriting writeData:[NSJSONSerialization dataWithJSONObject:request options:0 error:nil]];
            [input.fileHandleForWriting closeFile];
            NSData *data = [output.fileHandleForReading readDataOfLength:8193];
            [task waitUntilExit];
            id result = data.length <= 8192 ? [NSJSONSerialization JSONObjectWithData:data options:NSJSONReadingFragmentsAllowed error:nil] : nil;
            if (result) return json(result);
        } @catch (NSException *exception) {
            if (task.running) [task terminate];
        }
        return failure(@"The signed credential helper could not complete the request");
    }
}
char *buddy_autostart(const char *executable, bool enabled) {
    @autoreleasepool {
        NSString *directory = [NSHomeDirectory() stringByAppendingPathComponent:@"Library/LaunchAgents"];
        NSString *path = [directory stringByAppendingPathComponent:@"com.artmarketvm.desktopbuddy.plist"];
        if (!enabled) {
            if (![NSFileManager.defaultManager fileExistsAtPath:path]) return json(NSNull.null);
            return [NSFileManager.defaultManager removeItemAtPath:path error:nil] ? json(NSNull.null) : failure(@"Could not remove sumi login startup");
        }
        NSString *binary = @(executable);
        if (![binary containsString:@".app/Contents/MacOS/"]) return failure(@"Move sumi.app to Applications before enabling login startup");
        NSDictionary *agent = @{@"Label": @"com.artmarketvm.desktopbuddy", @"ProgramArguments": @[binary, @"--background"], @"RunAtLoad": @YES, @"LimitLoadToSessionType": @"Aqua"};
        NSData *data = [NSPropertyListSerialization dataWithPropertyList:agent format:NSPropertyListXMLFormat_v1_0 options:0 error:nil];
        if (![NSFileManager.defaultManager createDirectoryAtPath:directory withIntermediateDirectories:YES attributes:nil error:nil] || ![data writeToFile:path atomically:YES]) return failure(@"Could not save sumi login startup");
        return json(NSNull.null);
    }
}
// Carbon hotkeys are delivered on the native event thread; Rust immediately dispatches work.
static void (*shortcutCallback)(unsigned int) = NULL;
static OSStatus hotkey(EventHandlerCallRef next, EventRef event, void *context) {
    (void)next; (void)context;
    EventHotKeyID identity;
    if (GetEventParameter(event, kEventParamDirectObject, typeEventHotKeyID, NULL, sizeof(identity), NULL, &identity) == noErr && shortcutCallback) shortcutCallback(identity.id);
    return noErr;
}
unsigned int buddy_shortcuts(void (*callback)(unsigned int)) {
    shortcutCallback = callback;
    EventTypeSpec type = {kEventClassKeyboard, kEventHotKeyPressed};
    if (InstallEventHandler(GetApplicationEventTarget(), hotkey, 1, &type, NULL, NULL) != noErr) return 0;
    EventHotKeyRef buddy = NULL, goal = NULL;
    unsigned int available = 0;
    if (RegisterEventHotKey(kVK_ANSI_B, cmdKey | optionKey, (EventHotKeyID){0x42444459, 1}, GetApplicationEventTarget(), 0, &buddy) == noErr) available |= 1;
    if (RegisterEventHotKey(kVK_ANSI_G, cmdKey | optionKey, (EventHotKeyID){0x42444459, 2}, GetApplicationEventTarget(), 0, &goal) == noErr) available |= 2;
    return available;
}
char *buddy_voice_languages(void) {
    @autoreleasepool {
        NSMutableArray *languages = [NSMutableArray array];
        for (NSString *language in @[@"en-US", @"ru-RU"]) {
            SFSpeechRecognizer *recognizer = [[SFSpeechRecognizer alloc] initWithLocale:[NSLocale localeWithLocaleIdentifier:language]];
            if (recognizer.supportsOnDeviceRecognition) [languages addObject:language];
        }
        return json(languages);
    }
}
char *buddy_transcribe(const unsigned char *audio, size_t length, const char *language) {
    @autoreleasepool {
        if (NSThread.isMainThread) return failure(@"Start voice recognition from the workspace");
        dispatch_semaphore_t permission = dispatch_semaphore_create(0);
        __block SFSpeechRecognizerAuthorizationStatus authorization = SFSpeechRecognizer.authorizationStatus;
        if (authorization == SFSpeechRecognizerAuthorizationStatusNotDetermined) {
            dispatch_async(dispatch_get_main_queue(), ^{
                [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status) { authorization = status; dispatch_semaphore_signal(permission); }];
            });
            if (dispatch_semaphore_wait(permission, dispatch_time(DISPATCH_TIME_NOW, 30 * NSEC_PER_SEC)) != 0) return failure(@"Allow speech recognition in System Settings, then try again");
        }
        if (authorization != SFSpeechRecognizerAuthorizationStatusAuthorized) return failure(@"Allow sumi speech recognition in System Settings > Privacy & Security");
        NSString *choice = @(language);
        NSString *locale = [choice isEqualToString:@"ru"] ? @"ru-RU" : [choice isEqualToString:@"en"] ? @"en-US" : NSLocale.currentLocale.localeIdentifier;
        SFSpeechRecognizer *recognizer = [[SFSpeechRecognizer alloc] initWithLocale:[NSLocale localeWithLocaleIdentifier:locale]];
        if (!recognizer.supportsOnDeviceRecognition) return failure(@"Local speech recognition is unavailable for this language. Enable Dictation for it in System Settings > Keyboard, or paste a transcript.");
        NSString *path = [NSTemporaryDirectory() stringByAppendingPathComponent:[NSUUID.UUID.UUIDString stringByAppendingPathExtension:@"wav"]];
        NSURL *url = [NSURL fileURLWithPath:path];
        if (![[NSData dataWithBytes:audio length:length] writeToURL:url options:NSDataWritingAtomic | NSDataWritingFileProtectionComplete error:nil]) return failure(@"Could not prepare the voice recording");
        chmod(path.fileSystemRepresentation, 0600);
        SFSpeechURLRecognitionRequest *request = [[SFSpeechURLRecognitionRequest alloc] initWithURL:url];
        request.requiresOnDeviceRecognition = YES;
        request.shouldReportPartialResults = NO;
        dispatch_semaphore_t complete = dispatch_semaphore_create(0);
        __block NSString *transcript = nil;
        SFSpeechRecognitionTask *task = [recognizer recognitionTaskWithRequest:request resultHandler:^(SFSpeechRecognitionResult *result, NSError *error) {
            if (result.isFinal || error) { transcript = result.isFinal ? result.bestTranscription.formattedString : nil; dispatch_semaphore_signal(complete); }
        }];
        dispatch_semaphore_wait(complete, dispatch_time(DISPATCH_TIME_NOW, 75 * NSEC_PER_SEC));
        [task cancel];
        [NSFileManager.defaultManager removeItemAtURL:url error:nil];
        return transcript.length ? json(transcript) : failure(@"No local transcript was available. Check the installed Dictation language, try again, or paste a transcript.");
    }
}

// Capture one desktop-independent window. Never include the desktop or other windows.
bool buddy_screen_permission(bool request) {
    return CGPreflightScreenCaptureAccess() || (request && CGRequestScreenCaptureAccess());
}
// Capture failures carry a stable category code and a stage detail; neither contains screen content.
static char *captureFailure(NSString *code, NSString *detail) { return json(@{@"error": @"Window capture unavailable", @"code": code, @"detail": detail}); }
static BOOL sameFrame(CGRect a, CGRect b) {
    return fabs(a.origin.x - b.origin.x) <= 1 && fabs(a.origin.y - b.origin.y) <= 1
        && fabs(a.size.width - b.size.width) <= 1 && fabs(a.size.height - b.size.height) <= 1;
}
// Accessibility identities contain PID + AX hash, not a CGWindowID. The window list is ordered
// front to back, so the frontmost normal-level window of the focused application whose bounds
// match the focused Accessibility window is the focused window itself. Titles are not required
// to agree because the Accessibility title can lag behind a live window name; when several
// stacked windows share the same bounds, a matching title breaks the tie before z-order.
static CGWindowID focusedWindowNumber(pid_t pid, CGRect expectedFrame, NSString *expectedTitle) {
    NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID));
    CGWindowID frontmost = kCGNullWindowID, titled = kCGNullWindowID;
    for (NSDictionary *info in windows) {
        if ([info[(__bridge NSString *)kCGWindowOwnerPID] intValue] != pid || [info[(__bridge NSString *)kCGWindowLayer] intValue] != 0) continue;
        CGRect bounds = CGRectNull;
        if (!CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)info[(__bridge NSString *)kCGWindowBounds], &bounds) || !sameFrame(bounds, expectedFrame)) continue;
        CGWindowID number = [info[(__bridge NSString *)kCGWindowNumber] unsignedIntValue];
        if (frontmost == kCGNullWindowID) frontmost = number;
        if (titled == kCGNullWindowID && expectedTitle.length && [text(info[(__bridge NSString *)kCGWindowName], 160) isEqualToString:expectedTitle]) titled = number;
    }
    return titled != kCGNullWindowID ? titled : frontmost;
}
// The application's window carrying this identity, when it still exists (focused or not).
static AXUIElementRef windowWithIdentity(pid_t pid, uint64_t identity) {
    AXUIElementRef application = AXUIElementCreateApplication(pid);
    AXUIElementSetMessagingTimeout(application, 0.08);
    CFArrayRef windows = NULL;
    AXUIElementRef match = NULL;
    if (AXUIElementCopyAttributeValues(application, kAXWindowsAttribute, 0, 32, &windows) == kAXErrorSuccess) {
        for (id window in (__bridge NSArray *)windows) {
            if (windowID(pid, (__bridge AXUIElementRef)window) == identity) { match = (AXUIElementRef)CFRetain((__bridge CFTypeRef)window); break; }
        }
        CFRelease(windows);
    }
    CFRelease(application);
    return match;
}
// The focused Accessibility window must still carry the captured identity and bounds; the title
// may legitimately change while the capture runs, so it is not compared here.
static BOOL focusedWindowUnchanged(pid_t pid, uint64_t identity, CGRect expectedFrame) {
    AXUIElementRef latest = focusedWindow(pid);
    BOOL unchanged = latest && windowID(pid, latest) == identity && sameFrame(elementFrame(latest), expectedFrame);
    if (latest) CFRelease(latest);
    return unchanged;
}
char *buddy_capture_window(uint64_t identity) {
    @autoreleasepool {
        if (!buddy_session_active()) return captureFailure(@"session_inactive", @"session_inactive_or_locked");
        if (!CGPreflightScreenCaptureAccess()) return captureFailure(@"permission_unavailable", @"screen_recording_not_granted");
        pid_t pid = (pid_t)(identity >> 32);
        // A zero hash means the collector could not resolve a focused window (Accessibility
        // timeout or an application without windows); it never identifies a capture target.
        if ((uint32_t)identity == 0) return captureFailure(@"window_unavailable", @"focused_window_identity_unresolved");
        if (NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier != pid) return captureFailure(@"foreground_changed", @"frontmost_application_differs");
        AXUIElementRef focused = focusedWindow(pid);
        if (!focused || windowID(pid, focused) != identity) {
            if (focused) CFRelease(focused);
            AXUIElementRef known = windowWithIdentity(pid, identity);
            if (!known) return captureFailure(@"foreground_changed", @"target_window_closed_or_stale");
            BOOL minimized = [attribute(known, kAXMinimizedAttribute) boolValue];
            CFRelease(known);
            return minimized ? captureFailure(@"window_minimized", @"target_window_minimized") : captureFailure(@"foreground_changed", @"focused_window_differs");
        }
        if ([attribute(focused, kAXMinimizedAttribute) boolValue]) { CFRelease(focused); return captureFailure(@"window_minimized", @"focused_window_minimized"); }
        CGRect expectedFrame = elementFrame(focused);
        NSString *expectedTitle = text(attribute(focused, kAXTitleAttribute), 160);
        CFRelease(focused);
        if (CGRectIsNull(expectedFrame) || expectedFrame.size.width < 2 || expectedFrame.size.height < 2) return captureFailure(@"window_unavailable", @"focused_window_bounds_unavailable");
        CGWindowID number = focusedWindowNumber(pid, expectedFrame, expectedTitle);
        if (number == kCGNullWindowID) return captureFailure(@"window_unavailable", @"focused_window_not_on_screen");
        dispatch_group_t group = dispatch_group_create();
        dispatch_group_enter(group);
        __block NSData *jpeg = nil;
        __block NSString *code = @"api_error", *detail = @"shareable_content_unavailable";
        [SCShareableContent getShareableContentExcludingDesktopWindows:YES onScreenWindowsOnly:YES completionHandler:^(SCShareableContent *content, NSError *error) {
            SCWindow *selected = nil;
            for (SCWindow *window in content.windows) {
                if (window.windowID == number && window.owningApplication.processID == pid && sameFrame(window.frame, expectedFrame)) { selected = window; break; }
            }
            if (error) { dispatch_group_leave(group); return; }
            if (!selected) { detail = @"capture_window_not_resolved"; code = @"window_unavailable"; dispatch_group_leave(group); return; }
            // The desktop-independent filter renders this window's own layers only.
            SCContentFilter *filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:selected];
            SCStreamConfiguration *config = [SCStreamConfiguration new];
            double scale = MIN(1.0, 1280.0 / MAX(selected.frame.size.width, selected.frame.size.height));
            config.width = MAX(1, selected.frame.size.width * scale);
            config.height = MAX(1, selected.frame.size.height * scale);
            config.showsCursor = NO;
            config.ignoreShadowsSingleWindow = YES;
            [SCScreenshotManager captureImageWithFilter:filter configuration:config completionHandler:^(CGImageRef image, NSError *captureError) {
                if (captureError || !image) { detail = @"screenshot_manager_failed"; }
                else if (!buddy_session_active()) { code = @"session_inactive"; detail = @"session_locked_during_capture"; }
                else if (NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier != pid) { code = @"foreground_changed"; detail = @"frontmost_application_changed_during_capture"; }
                else if (!focusedWindowUnchanged(pid, identity, expectedFrame)) { code = @"foreground_changed"; detail = @"focused_window_changed_during_capture"; }
                else {
                    NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc] initWithCGImage:image];
                    NSData *data = [bitmap representationUsingType:NSBitmapImageFileTypeJPEG properties:@{NSImageCompressionFactor:@0.65}];
                    if (!data) { code = @"encoding_failed"; detail = @"jpeg_encoding_failed"; }
                    else if (data.length > 300000) { code = @"image_too_large"; detail = @"jpeg_over_budget"; }
                    else jpeg = data;
                }
                dispatch_group_leave(group);
            }];
        }];
        if (dispatch_group_wait(group, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC)) != 0) return captureFailure(@"capture_timeout", @"screenshot_wait_timed_out");
        if (!jpeg) return captureFailure(code, detail);
        return json([jpeg base64EncodedStringWithOptions:0]);
    }
}
