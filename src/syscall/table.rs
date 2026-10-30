//! The closed system call list of contract 3.10, by number (`/usr/include/asm/unistd_64.h`).

/// How a guest system call is served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Straight to the kernel with the same arguments.
    Pass,
    /// Handled by calco.
    Emulated,
    /// Not in the contract list: `-ENOSYS`, noted in `CALCO_LOG`.
    Unlisted,
}

pub const MMAP: u64 = 9;
pub const MPROTECT: u64 = 10;
pub const MUNMAP: u64 = 11;
pub const BRK: u64 = 12;
pub const RT_SIGACTION: u64 = 13;
pub const RT_SIGPROCMASK: u64 = 14;
pub const RT_SIGRETURN: u64 = 15;
pub const MREMAP: u64 = 25;
pub const MADVISE: u64 = 28;
pub const SHMAT: u64 = 30;
pub const CLONE: u64 = 56;
pub const FORK: u64 = 57;
pub const VFORK: u64 = 58;
pub const EXECVE: u64 = 59;
pub const EXIT: u64 = 60;
pub const SHMDT: u64 = 67;
pub const WAIT4: u64 = 61;
pub const KILL: u64 = 62;
pub const READLINK: u64 = 89;
pub const RT_SIGPENDING: u64 = 127;
pub const RT_SIGTIMEDWAIT: u64 = 128;
pub const RT_SIGQUEUEINFO: u64 = 129;
pub const RT_SIGSUSPEND: u64 = 130;
pub const SIGALTSTACK: u64 = 131;
pub const PRCTL: u64 = 157;
pub const ARCH_PRCTL: u64 = 158;
pub const GETTID: u64 = 186;
pub const TKILL: u64 = 200;
pub const SET_TID_ADDRESS: u64 = 218;
pub const TIMER_CREATE: u64 = 222;
pub const WAITID: u64 = 247;
pub const EXIT_GROUP: u64 = 231;
pub const TGKILL: u64 = 234;
pub const READLINKAT: u64 = 267;
pub const SET_ROBUST_LIST: u64 = 273;
pub const GET_ROBUST_LIST: u64 = 274;
pub const SIGNALFD4: u64 = 289;
pub const RT_TGSIGQUEUEINFO: u64 = 297;
pub const EXECVEAT: u64 = 322;
pub const RSEQ: u64 = 334;
pub const CLONE3: u64 = 435;

/// Emulated calls. `prctl` passes only for the options the contract names; `readlink`
/// and `readlinkat` pass except for `/proc/self/exe`; `wait4`, `waitid` and `madvise`
/// pass and then translate the signal of a wait status or drop stale translations;
/// `timer_create` passes with the signal of its `sigevent` translated.
pub const EMULATED: &[(u64, &str)] = &[
    (MMAP, "mmap"),
    (MPROTECT, "mprotect"),
    (MUNMAP, "munmap"),
    (BRK, "brk"),
    (RT_SIGACTION, "rt_sigaction"),
    (RT_SIGPROCMASK, "rt_sigprocmask"),
    (RT_SIGRETURN, "rt_sigreturn"),
    (MREMAP, "mremap"),
    (MADVISE, "madvise"),
    (SHMAT, "shmat"),
    (CLONE, "clone"),
    (FORK, "fork"),
    (VFORK, "vfork"),
    (EXECVE, "execve"),
    (EXIT, "exit"),
    (SHMDT, "shmdt"),
    (WAIT4, "wait4"),
    (KILL, "kill"),
    (READLINK, "readlink"),
    (RT_SIGPENDING, "rt_sigpending"),
    (RT_SIGTIMEDWAIT, "rt_sigtimedwait"),
    (RT_SIGQUEUEINFO, "rt_sigqueueinfo"),
    (RT_SIGSUSPEND, "rt_sigsuspend"),
    (SIGALTSTACK, "sigaltstack"),
    (PRCTL, "prctl"),
    (ARCH_PRCTL, "arch_prctl"),
    (TKILL, "tkill"),
    (SET_TID_ADDRESS, "set_tid_address"),
    (WAITID, "waitid"),
    (EXIT_GROUP, "exit_group"),
    (TGKILL, "tgkill"),
    (READLINKAT, "readlinkat"),
    (SET_ROBUST_LIST, "set_robust_list"),
    (GET_ROBUST_LIST, "get_robust_list"),
    (SIGNALFD4, "signalfd4"),
    (TIMER_CREATE, "timer_create"),
    (RT_TGSIGQUEUEINFO, "rt_tgsigqueueinfo"),
    (EXECVEAT, "execveat"),
    (RSEQ, "rseq"),
    (CLONE3, "clone3"),
];

/// Calls passed straight to the kernel.
pub const PASS: &[(u64, &str)] = &[
    (0, "read"),
    (1, "write"),
    (19, "readv"),
    (20, "writev"),
    (17, "pread64"),
    (18, "pwrite64"),
    (295, "preadv"),
    (296, "pwritev"),
    (257, "openat"),
    (2, "open"),
    (3, "close"),
    (436, "close_range"),
    (8, "lseek"),
    (5, "fstat"),
    (4, "stat"),
    (6, "lstat"),
    (262, "newfstatat"),
    (332, "statx"),
    (137, "statfs"),
    (138, "fstatfs"),
    (21, "access"),
    (269, "faccessat"),
    (439, "faccessat2"),
    (217, "getdents64"),
    (79, "getcwd"),
    (80, "chdir"),
    (81, "fchdir"),
    (83, "mkdir"),
    (258, "mkdirat"),
    (84, "rmdir"),
    (87, "unlink"),
    (263, "unlinkat"),
    (82, "rename"),
    (264, "renameat"),
    (316, "renameat2"),
    (86, "link"),
    (265, "linkat"),
    (88, "symlink"),
    (266, "symlinkat"),
    (90, "chmod"),
    (91, "fchmod"),
    (268, "fchmodat"),
    (92, "chown"),
    (93, "fchown"),
    (260, "fchownat"),
    (94, "lchown"),
    (280, "utimensat"),
    (261, "futimesat"),
    (76, "truncate"),
    (77, "ftruncate"),
    (74, "fsync"),
    (75, "fdatasync"),
    (285, "fallocate"),
    (221, "fadvise64"),
    (162, "sync"),
    (95, "umask"),
    (32, "dup"),
    (33, "dup2"),
    (292, "dup3"),
    (22, "pipe"),
    (293, "pipe2"),
    (72, "fcntl"),
    (73, "flock"),
    (16, "ioctl"),
    (7, "poll"),
    (271, "ppoll"),
    (23, "select"),
    (270, "pselect6"),
    (291, "epoll_create1"),
    (233, "epoll_ctl"),
    (232, "epoll_wait"),
    (281, "epoll_pwait"),
    (290, "eventfd2"),
    (283, "timerfd_create"),
    (286, "timerfd_settime"),
    (287, "timerfd_gettime"),
    (319, "memfd_create"),
    (326, "copy_file_range"),
    (40, "sendfile"),
    (275, "splice"),
    (41, "socket"),
    (53, "socketpair"),
    (42, "connect"),
    (49, "bind"),
    (50, "listen"),
    (43, "accept"),
    (288, "accept4"),
    (44, "sendto"),
    (45, "recvfrom"),
    (46, "sendmsg"),
    (47, "recvmsg"),
    (48, "shutdown"),
    (55, "getsockopt"),
    (54, "setsockopt"),
    (51, "getsockname"),
    (52, "getpeername"),
    (39, "getpid"),
    (110, "getppid"),
    (GETTID, "gettid"),
    (102, "getuid"),
    (107, "geteuid"),
    (104, "getgid"),
    (108, "getegid"),
    (115, "getgroups"),
    (105, "setuid"),
    (106, "setgid"),
    (113, "setreuid"),
    (114, "setregid"),
    (117, "setresuid"),
    (119, "setresgid"),
    (118, "getresuid"),
    (120, "getresgid"),
    (109, "setpgid"),
    (121, "getpgid"),
    (111, "getpgrp"),
    (112, "setsid"),
    (124, "getsid"),
    (98, "getrusage"),
    (100, "times"),
    (63, "uname"),
    (99, "sysinfo"),
    (302, "prlimit64"),
    (97, "getrlimit"),
    (160, "setrlimit"),
    (140, "getpriority"),
    (141, "setpriority"),
    (24, "sched_yield"),
    (204, "sched_getaffinity"),
    (203, "sched_setaffinity"),
    (143, "sched_getparam"),
    (145, "sched_getscheduler"),
    (228, "clock_gettime"),
    (229, "clock_getres"),
    (230, "clock_nanosleep"),
    (35, "nanosleep"),
    (96, "gettimeofday"),
    (201, "time"),
    (318, "getrandom"),
    (202, "futex"),
    (37, "alarm"),
    (38, "setitimer"),
    (36, "getitimer"),
    (223, "timer_settime"),
    (224, "timer_gettime"),
    (226, "timer_delete"),
    (324, "membarrier"),
    (149, "mlock"),
    (150, "munlock"),
    (27, "mincore"),
    (26, "msync"),
    (294, "inotify_init1"),
    (254, "inotify_add_watch"),
    (255, "inotify_rm_watch"),
    (29, "shmget"),
    (31, "shmctl"),
];

pub fn classify(nr: u64) -> Kind {
    if PASS.iter().any(|&(n, _)| n == nr) {
        Kind::Pass
    } else if EMULATED.iter().any(|&(n, _)| n == nr) {
        Kind::Emulated
    } else {
        Kind::Unlisted
    }
}
