// Loaded into a test instance only (DYLD_INSERT_LIBRARIES, set by
// sifs-verify launch). Opening a result runs /usr/bin/open and showing it in
// Finder calls NSWorkspace; either would bring another app forward and could
// open the owner's apps. This records each request in $SIFS_VERIFY_OPEN_LOG
// and reports success without opening anything.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <spawn.h>
#include <string.h>

static void record(NSString *line) {
    const char *path = getenv("SIFS_VERIFY_OPEN_LOG");
    if (!path) return;
    FILE *log = fopen(path, "a");
    if (!log) return;
    fprintf(log, "%s\n", line.UTF8String);
    fclose(log);
}

static int spy_spawn(pid_t *pid, const char *path, const posix_spawn_file_actions_t *actions,
                     const posix_spawnattr_t *attrs, char *const argv[], char *const envp[]) {
    if (path && strcmp(path, "/usr/bin/open") == 0) {
        NSMutableArray *words = [NSMutableArray arrayWithObject:@"open"];
        for (int i = 1; argv[i]; i++) [words addObject:@(argv[i])];
        record([words componentsJoinedByString:@"\t"]);
        char *const instead[] = {"/usr/bin/true", NULL};
        return posix_spawn(pid, "/usr/bin/true", actions, attrs, instead, envp);
    }
    return posix_spawn(pid, path, actions, attrs, argv, envp);
}

static int spy_spawnp(pid_t *pid, const char *file, const posix_spawn_file_actions_t *actions,
                      const posix_spawnattr_t *attrs, char *const argv[], char *const envp[]) {
    if (file && strcmp(file, "/usr/bin/open") == 0) return spy_spawn(pid, file, actions, attrs, argv, envp);
    return posix_spawnp(pid, file, actions, attrs, argv, envp);
}

__attribute__((used, section("__DATA,__interpose"))) static struct {
    const void *replacement, *original;
} interposers[] = {
    {(const void *)spy_spawn, (const void *)posix_spawn},
    {(const void *)spy_spawnp, (const void *)posix_spawnp},
};

__attribute__((constructor)) static void spy_reveal(void) {
    Method method = class_getInstanceMethod([NSWorkspace class], @selector(activateFileViewerSelectingURLs:));
    method_setImplementation(method, imp_implementationWithBlock(^(id workspace, NSArray<NSURL *> *urls) {
        NSMutableArray *words = [NSMutableArray arrayWithObject:@"reveal"];
        for (NSURL *url in urls) [words addObject:url.path];
        record([words componentsJoinedByString:@"\t"]);
    }));
    record(@"spy loaded");
}
