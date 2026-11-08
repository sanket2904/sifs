// Loaded into a test instance only (DYLD_INSERT_LIBRARIES, set by
// sifs-verify launch). Opening a result runs /usr/bin/open and showing it in
// Finder calls NSWorkspace; either would bring another app forward and could
// open the owner's apps. This records each request in the log launch created,
// through the descriptor $SIFS_VERIFY_OPEN_LOG_FD it inherits, and reports
// success without opening anything. It never opens the log by path.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <fcntl.h>
#include <spawn.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int log_fd = -1;

static void record(NSString *line) {
    if (log_fd >= 0) dprintf(log_fd, "%s\n", line.UTF8String);
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
    const char *fd = getenv("SIFS_VERIFY_OPEN_LOG_FD");
    struct stat st;
    if (fd && fstat(atoi(fd), &st) == 0 && S_ISREG(st.st_mode)) {
        log_fd = atoi(fd);
        fcntl(log_fd, F_SETFD, FD_CLOEXEC);
    }
    Method method = class_getInstanceMethod([NSWorkspace class], @selector(activateFileViewerSelectingURLs:));
    method_setImplementation(method, imp_implementationWithBlock(^(id workspace, NSArray<NSURL *> *urls) {
        NSMutableArray *words = [NSMutableArray arrayWithObject:@"reveal"];
        for (NSURL *url in urls) [words addObject:url.path];
        record([words componentsJoinedByString:@"\t"]);
    }));
    record(@"spy loaded");
}
