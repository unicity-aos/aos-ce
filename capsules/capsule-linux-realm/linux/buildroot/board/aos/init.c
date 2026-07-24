#define _GNU_SOURCE

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <grp.h>
#include <signal.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/reboot.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/mount.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

#define PROTOCOL_PREFIX "AOS/1 "
#define TOKEN_BYTES 16
#define TOKEN_HEX_BYTES (TOKEN_BYTES * 2)
#define MAX_COMMAND_BYTES 1024
#define MAX_SCRIPT_BYTES (512U * 1024U)
#define MAX_BASE64_SCRIPT_BYTES (((MAX_SCRIPT_BYTES + 2U) / 3U) * 4U)
#define MAX_CWD_BYTES 64
#define PROTOCOL_OVERHEAD_BYTES 128
#define MAX_PROTOCOL_LINE_BYTES \
    (MAX_BASE64_SCRIPT_BYTES + MAX_CWD_BYTES + PROTOCOL_OVERHEAD_BYTES)
#define MAX_FILE_BYTES (1ULL << 40)
#define MAX_PROCESSES 65536UL
#define MAX_OPEN_FILES 1048576UL
#define AGENT_UID 1000
#define AGENT_GID 1000

static uint64_t command_count;
static int refresh_wall_clock(unsigned long long seconds);

static int set_limit(int resource, rlim_t value)
{
    struct rlimit limit = { .rlim_cur = value, .rlim_max = value };

    return setrlimit(resource, &limit);
}

static void write_all(const char *bytes, size_t length)
{
    while (length > 0) {
        ssize_t written = write(STDOUT_FILENO, bytes, length);
        if (written < 0 && errno == EINTR)
            continue;
        if (written <= 0)
            return;
        bytes += (size_t)written;
        length -= (size_t)written;
    }
}

static void write_text(const char *text)
{
    write_all(text, strlen(text));
}

static ssize_t read_protocol_line(char *line, size_t capacity)
{
    size_t length = 0;

    while (length < capacity) {
        ssize_t count = read(STDIN_FILENO, line + length, capacity - length);
        if (count < 0 && errno == EINTR)
            continue;
        if (count <= 0)
            return count;
        size_t previous = length;
        length += (size_t)count;
        for (size_t index = previous; index < length; index++) {
            if (line[index] != '\n')
                continue;
            if (index + 1 != length)
                return -2;
            return (ssize_t)length;
        }
    }

    /* Drain an oversized frame before accepting the next command. */
    char byte;
    for (;;) {
        ssize_t count = read(STDIN_FILENO, &byte, 1);
        if (count < 0 && errno == EINTR)
            continue;
        if (count <= 0 || byte == '\n')
            break;
    }
    return -2;
}

static int mount_bootstrap_devices(void)
{
    if (mount("devtmpfs", "/dev", "devtmpfs", MS_NOSUID | MS_NOEXEC,
              "mode=0755") < 0 && errno != EBUSY)
        return 70;
    return 0;
}

static int attach_immutable_system(void)
{
    if (mkdir("/system", 0755) < 0 && errno != EEXIST) {
        write_text("system mount point failed\n");
        return 70;
    }
    if (mount("/dev/aos-system", "/system", "squashfs",
              MS_RDONLY | MS_NOSUID | MS_NODEV, "") < 0) {
        write_text("immutable system mount failed\n");
        return 70;
    }
    if (mount("/dev", "/system/dev", NULL, MS_MOVE, NULL) < 0) {
        write_text("device filesystem move failed\n");
        return 70;
    }
    if (chdir("/system") < 0 || chroot(".") < 0 || chdir("/") < 0) {
        write_text("immutable system root failed\n");
        return 70;
    }
    return 0;
}

static int prepare_ephemeral_filesystems(void)
{
    if ((mkdir("/run", 0755) < 0 && errno != EEXIST) ||
        (mkdir("/tmp", 01777) < 0 && errno != EEXIST)) {
        write_text("ephemeral mount point failed\n");
        return 70;
    }
    if (mount("tmpfs", "/run", "tmpfs", MS_NOSUID | MS_NODEV,
              "mode=0755,size=64m") < 0 ||
        mount("tmpfs", "/tmp", "tmpfs", MS_NOSUID | MS_NODEV,
              "mode=1777,size=256m") < 0) {
        write_text("ephemeral filesystem mount failed\n");
        return 70;
    }
    return 0;
}

static bool valid_token(const char *token)
{
    for (size_t index = 0; index < TOKEN_HEX_BYTES; index++) {
        char byte = token[index];
        if (!((byte >= '0' && byte <= '9') || (byte >= 'a' && byte <= 'f')))
            return false;
    }
    return token[TOKEN_HEX_BYTES] == ' ';
}

static void emit_begin(const char *token)
{
    write_text("AOS BEGIN ");
    write_all(token, TOKEN_HEX_BYTES);
    write_text("\n");
}

static void emit_end(const char *token, int status)
{
    char status_text[16];
    int length = snprintf(status_text, sizeof(status_text), "%d", status);

    write_text("\nAOS END ");
    write_all(token, TOKEN_HEX_BYTES);
    write_text(" ");
    if (length > 0)
        write_all(status_text, (size_t)length);
    write_text("\n");
}

static int refresh_home(void)
{
    static const char options[] =
        "trans=aos,version=9p2000.L,msize=65536,cache=none,access=client,"
        "aname=home,noxattr,dfltuid=1000,dfltgid=1000";

    if (mkdir("/home/agent", 0700) < 0 && errno != EEXIST) {
        write_text("home mount point failed\n");
        return 70;
    }
    if (umount2("/home/agent", 0) < 0 && errno != EINVAL && errno != ENOENT) {
        write_text("home unmount failed\n");
        return 70;
    }
    if (mount("home", "/home/agent", "9p", MS_NOSUID | MS_NODEV,
              options) < 0) {
        write_text("home mount failed\n");
        return 70;
    }
    return 0;
}

static int refresh_workspace(void)
{
    static const char options[] =
        "trans=aos,version=9p2000.L,msize=65536,cache=none,access=client,"
        "aname=workspace,noxattr,dfltuid=1000,dfltgid=1000";

    if (mkdir("/workspace", 0700) < 0 && errno != EEXIST) {
        write_text("workspace mount point failed\n");
        return 70;
    }
    if (umount2("/workspace", 0) < 0 && errno != EINVAL && errno != ENOENT) {
        write_text("workspace unmount failed\n");
        return 70;
    }
    if (mount("workspace", "/workspace", "9p", MS_NOSUID | MS_NODEV,
              options) < 0) {
        write_text("workspace mount failed\n");
        return 70;
    }
    return 0;
}

static int prepare_toolchain_runtime(void)
{
    if ((mkdir("/run", 0755) < 0 && errno != EEXIST) ||
        (mkdir("/run/aos", 0755) < 0 && errno != EEXIST) ||
        (mkdir("/run/aos/rustup", 0700) < 0 && errno != EEXIST) ||
        (mkdir("/run/aos/cargo", 0700) < 0 && errno != EEXIST) ||
        chmod("/run", 0755) < 0 || chmod("/run/aos", 0755) < 0 ||
        chmod("/run/aos/rustup", 0700) < 0 ||
        chmod("/run/aos/cargo", 0700) < 0 ||
        chown("/run/aos/rustup", AGENT_UID, AGENT_GID) < 0 ||
        chown("/run/aos/cargo", AGENT_UID, AGENT_GID) < 0) {
        write_text("toolchain runtime home failed\n");
        return 70;
    }
    return 0;
}

static int shell_status(const char *cwd, const char *script,
                        rlim_t max_file_bytes, rlim_t max_processes,
                        rlim_t max_open_files)
{
    pid_t child = fork();
    if (child < 0)
        return 70;

    if (child == 0) {
        char *const argv[] = {
            "bash", "--noprofile", "--norc", "-c", (char *)script, NULL
        };
        static char *const environment[] = {
            "HOME=/home/agent",
            "PATH=/home/agent/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            "USER=agent",
            "LOGNAME=agent",
            "SHELL=/bin/bash",
            "TERM=dumb",
            "LANG=C",
            "LC_ALL=C",
            "TMPDIR=/tmp",
            "CARGO_HOME=/run/aos/cargo",
            "CARGO_INSTALL_ROOT=/home/agent/.cargo",
            "RUSTUP_HOME=/run/aos/rustup",
            "RUSTUP_TOOLCHAIN=aos-system",
            "RUST_BACKTRACE=1",
            "BASH_ENV=/usr/libexec/aos/realm-env",
            "CC=cc",
            "CXX=c++",
            "AR=ar",
            "AOS_REALM_NAME=AOS Realm",
            "AOS_PRINCIPAL_HOME=/home/agent",
            "AOS_WORKSPACE=/workspace",
            NULL,
        };
        int null_fd;
        int output_fd;

        (void)setpgid(0, 0);
        null_fd = open("/dev/null", O_RDONLY | O_CLOEXEC);
        output_fd = open("/dev/console", O_WRONLY | O_NOCTTY | O_CLOEXEC);
        if (null_fd < 0 || output_fd < 0 || dup2(null_fd, STDIN_FILENO) < 0 ||
            dup2(output_fd, STDOUT_FILENO) < 0 ||
            dup2(output_fd, STDERR_FILENO) < 0)
            _exit(126);
        if (null_fd != STDIN_FILENO)
            close(null_fd);
        if (output_fd != STDOUT_FILENO && output_fd != STDERR_FILENO)
            close(output_fd);
        if (chdir(cwd) < 0 || setgroups(0, NULL) < 0 ||
            setgid(AGENT_GID) < 0 || setuid(AGENT_UID) < 0)
            _exit(126);
        if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0 ||
            set_limit(RLIMIT_CORE, 0) < 0 ||
            (max_open_files != 0 && set_limit(RLIMIT_NOFILE, max_open_files) < 0) ||
            (max_processes != 0 && set_limit(RLIMIT_NPROC, max_processes) < 0) ||
            set_limit(RLIMIT_FSIZE,
                      max_file_bytes == 0 ? RLIM_INFINITY : max_file_bytes) < 0)
            _exit(126);
        execve("/bin/bash", argv, environment);
        _exit(127);
    }

    (void)setpgid(child, child);
    int child_status;
    while (waitpid(child, &child_status, 0) < 0) {
        if (errno != EINTR)
            return 70;
    }

    /* PID 1 owns the whole guest process lane. Kill and reap every orphan
     * before emitting the result frame so background output cannot cross a
     * command boundary. */
    (void)kill(-1, SIGKILL);
    int orphan_status;
    while (waitpid(-1, &orphan_status, 0) > 0)
        ;

    if (WIFEXITED(child_status))
        return WEXITSTATUS(child_status);
    if (WIFSIGNALED(child_status))
        return 128 + WTERMSIG(child_status);
    return 70;
}

static bool has_path_component(const char *path, const char *component)
{
    size_t component_length = strlen(component);
    const char *cursor = path;

    while (*cursor != '\0') {
        while (*cursor == '/')
            cursor++;
        const char *start = cursor;
        while (*cursor != '\0' && *cursor != '/')
            cursor++;
        if ((size_t)(cursor - start) == component_length &&
            memcmp(start, component, component_length) == 0)
            return true;
    }
    return false;
}

static bool valid_shell_cwd(const char *cwd)
{
    static const char home[] = "/home/agent";
    static const char workspace[] = "/workspace";

    for (const unsigned char *cursor = (const unsigned char *)cwd;
         *cursor != '\0'; cursor++) {
        if (*cursor < 0x20 || *cursor == 0x7f || *cursor == '\\')
            return false;
    }
    if (strstr(cwd, "//") != NULL || has_path_component(cwd, ".") ||
        has_path_component(cwd, ".."))
        return false;
    return strcmp(cwd, home) == 0 ||
           (strncmp(cwd, home, sizeof(home) - 1) == 0 &&
            cwd[sizeof(home) - 1] == '/') ||
           strcmp(cwd, workspace) == 0 ||
           (strncmp(cwd, workspace, sizeof(workspace) - 1) == 0 &&
            cwd[sizeof(workspace) - 1] == '/');
}

static int shell_command_status(char *payload)
{
    char *file_limit_end;
    char *first_limit_end;
    char *second_limit_end;
    char *length_end;
    char *cwd;
    char *script;
    unsigned long cwd_length;
    unsigned long first_limit;
    unsigned long second_limit;
    unsigned long long max_file_bytes;
    unsigned long max_processes;
    unsigned long max_open_files;
    size_t remaining;

    if (*payload < '0' || *payload > '9')
        return 64;
    errno = 0;
    max_file_bytes = strtoull(payload, &file_limit_end, 10);
    if (errno != 0 || file_limit_end == payload || *file_limit_end != ' ' ||
        max_file_bytes > MAX_FILE_BYTES)
        return 64;
    payload = file_limit_end + 1;
    errno = 0;
    first_limit = strtoul(payload, &first_limit_end, 10);
    if (errno != 0 || first_limit_end == payload || *first_limit_end != ' ')
        return 64;
    payload = first_limit_end + 1;
    if (*payload >= '0' && *payload <= '9') {
        max_processes = first_limit;
        if (max_processes > MAX_PROCESSES)
            return 64;
        errno = 0;
        second_limit = strtoul(payload, &second_limit_end, 10);
        if (errno != 0 || second_limit_end == payload || *second_limit_end != ' ')
            return 64;
        payload = second_limit_end + 1;
        if (*payload >= '0' && *payload <= '9') {
            max_open_files = second_limit;
            if (max_open_files > MAX_OPEN_FILES)
                return 64;
            errno = 0;
            cwd_length = strtoul(payload, &length_end, 10);
            if (errno != 0 || length_end == payload || *length_end != ' ')
                return 64;
            cwd = length_end + 1;
        } else {
            max_open_files = 0;
            cwd_length = second_limit;
            cwd = payload;
        }
    } else {
        /* Original frame: the value after max-file-bytes is cwd length. */
        max_processes = 0;
        max_open_files = 0;
        cwd_length = first_limit;
        cwd = payload;
    }
    if (cwd_length == 0 || cwd_length > MAX_CWD_BYTES)
        return 64;
    remaining = strlen(cwd);
    if (remaining <= cwd_length || cwd[cwd_length] != ' ')
        return 64;
    cwd[cwd_length] = '\0';
    script = cwd + cwd_length + 1;
    if (!valid_shell_cwd(cwd) || *script == '\0')
        return 64;
    int status = refresh_home();
    if (status != 0)
        return status;
    status = refresh_workspace();
    if (status != 0)
        return status;
    return shell_status(cwd, script, (rlim_t)max_file_bytes,
                        (rlim_t)max_processes, (rlim_t)max_open_files);
}

static bool parse_u64_token(char **cursor, unsigned long long maximum,
                            unsigned long long *value)
{
    char *end;

    if (**cursor < '0' || **cursor > '9')
        return false;
    errno = 0;
    *value = strtoull(*cursor, &end, 10);
    if (errno != 0 || end == *cursor || *end != ' ' || *value > maximum)
        return false;
    *cursor = end + 1;
    return true;
}

static int base64_digit(unsigned char byte)
{
    if (byte >= 'A' && byte <= 'Z')
        return byte - 'A';
    if (byte >= 'a' && byte <= 'z')
        return byte - 'a' + 26;
    if (byte >= '0' && byte <= '9')
        return byte - '0' + 52;
    if (byte == '+')
        return 62;
    if (byte == '/')
        return 63;
    return -1;
}

static char *decode_base64_script(const char *encoded, size_t script_length)
{
    size_t encoded_length = strlen(encoded);
    size_t expected_length = ((script_length + 2U) / 3U) * 4U;
    char *script;
    size_t input = 0;
    size_t output = 0;

    if (script_length == 0 || script_length > MAX_SCRIPT_BYTES ||
        encoded_length != expected_length)
        return NULL;
    script = malloc(script_length + 1U);
    if (script == NULL)
        return NULL;

    while (input < encoded_length) {
        int first = base64_digit((unsigned char)encoded[input]);
        int second = base64_digit((unsigned char)encoded[input + 1]);
        unsigned char third_byte = (unsigned char)encoded[input + 2];
        unsigned char fourth_byte = (unsigned char)encoded[input + 3];
        int third = third_byte == '=' ? 0 : base64_digit(third_byte);
        int fourth = fourth_byte == '=' ? 0 : base64_digit(fourth_byte);
        size_t remaining = script_length - output;
        bool final = input + 4U == encoded_length;

        if (first < 0 || second < 0 || third < 0 || fourth < 0 ||
            (!final && (third_byte == '=' || fourth_byte == '=')) ||
            (remaining >= 3U && (third_byte == '=' || fourth_byte == '=')) ||
            (remaining == 2U &&
             (third_byte == '=' || fourth_byte != '=' || (third & 0x03) != 0)) ||
            (remaining == 1U &&
             (third_byte != '=' || fourth_byte != '=' || (second & 0x0f) != 0)) ||
            remaining == 0) {
            free(script);
            return NULL;
        }

        script[output++] = (char)((first << 2) | (second >> 4));
        if (remaining > 1U)
            script[output++] = (char)((second << 4) | (third >> 2));
        if (remaining > 2U)
            script[output++] = (char)((third << 6) | fourth);
        input += 4U;
    }

    if (output != script_length || memchr(script, '\0', script_length) != NULL) {
        free(script);
        return NULL;
    }
    script[script_length] = '\0';
    return script;
}

static int shell_command_v2_status(char *payload)
{
    unsigned long long wall_seconds;
    unsigned long long max_file_bytes;
    unsigned long long max_processes;
    unsigned long long max_open_files;
    unsigned long long cwd_length;
    char *cwd;
    char *script;

    if (!parse_u64_token(&payload, (unsigned long long)INT64_MAX,
                         &wall_seconds) ||
        wall_seconds == 0 ||
        !parse_u64_token(&payload, MAX_FILE_BYTES, &max_file_bytes) ||
        !parse_u64_token(&payload, MAX_PROCESSES, &max_processes) ||
        !parse_u64_token(&payload, MAX_OPEN_FILES, &max_open_files) ||
        !parse_u64_token(&payload, MAX_CWD_BYTES, &cwd_length) ||
        cwd_length == 0)
        return 64;
    cwd = payload;
    if (strlen(cwd) <= cwd_length || cwd[cwd_length] != ' ')
        return 64;
    cwd[cwd_length] = '\0';
    script = cwd + cwd_length + 1;
    if (!valid_shell_cwd(cwd) || *script == '\0')
        return 64;
    int status = refresh_wall_clock(wall_seconds);
    if (status != 0)
        return status;
    status = refresh_home();
    if (status != 0)
        return status;
    status = refresh_workspace();
    if (status != 0)
        return status;
    return shell_status(cwd, script, (rlim_t)max_file_bytes,
                        (rlim_t)max_processes, (rlim_t)max_open_files);
}

static int shell_command_v3_status(char *payload)
{
    unsigned long long wall_seconds;
    unsigned long long max_file_bytes;
    unsigned long long max_processes;
    unsigned long long max_open_files;
    unsigned long long cwd_length;
    unsigned long long script_length;
    char *cwd;
    char *encoded;
    char *script;
    int status;

    if (!parse_u64_token(&payload, (unsigned long long)INT64_MAX,
                         &wall_seconds) ||
        wall_seconds == 0 ||
        !parse_u64_token(&payload, MAX_FILE_BYTES, &max_file_bytes) ||
        !parse_u64_token(&payload, MAX_PROCESSES, &max_processes) ||
        !parse_u64_token(&payload, MAX_OPEN_FILES, &max_open_files) ||
        !parse_u64_token(&payload, MAX_CWD_BYTES, &cwd_length) ||
        cwd_length == 0 ||
        !parse_u64_token(&payload, MAX_SCRIPT_BYTES, &script_length) ||
        script_length == 0)
        return 64;
    cwd = payload;
    if (strlen(cwd) <= cwd_length || cwd[cwd_length] != ' ')
        return 64;
    cwd[cwd_length] = '\0';
    encoded = cwd + cwd_length + 1;
    if (!valid_shell_cwd(cwd) || *encoded == '\0')
        return 64;
    script = decode_base64_script(encoded, (size_t)script_length);
    if (script == NULL)
        return 64;

    status = refresh_wall_clock(wall_seconds);
    if (status == 0)
        status = refresh_home();
    if (status == 0)
        status = refresh_workspace();
    if (status == 0)
        status = shell_status(cwd, script, (rlim_t)max_file_bytes,
                              (rlim_t)max_processes,
                              (rlim_t)max_open_files);
    free(script);
    return status;
}

static int execute_command(char *command)
{
    if (strcmp(command, "ping") == 0) {
        write_text("pong\n");
        return 0;
    }
    if (strcmp(command, "counter") == 0) {
        char number[32];
        int length = snprintf(number, sizeof(number), "counter=%llu\n",
                              (unsigned long long)command_count);
        if (length > 0)
            write_all(number, (size_t)length);
        return 0;
    }
    if (strncmp(command, "echo ", 5) == 0) {
        write_text(command + 5);
        write_text("\n");
        return 0;
    }
    if (strncmp(command, "sh ", 3) == 0 && command[3] != '\0')
        return shell_command_status(command + 3);
    if (strncmp(command, "sh2 ", 4) == 0 && command[4] != '\0')
        return shell_command_v2_status(command + 4);
    if (strncmp(command, "sh3 ", 4) == 0 && command[4] != '\0')
        return shell_command_v3_status(command + 4);
    write_text("unknown command\n");
    return 64;
}

static void configure_console(void)
{
    int console = open("/dev/console", O_RDWR | O_NOCTTY);
    if (console >= 0) {
        (void)dup2(console, STDIN_FILENO);
        (void)dup2(console, STDOUT_FILENO);
        (void)dup2(console, STDERR_FILENO);
        if (console > STDERR_FILENO)
            close(console);
    }

    struct termios attributes;
    if (tcgetattr(STDIN_FILENO, &attributes) == 0) {
        attributes.c_lflag &= (tcflag_t)~(ECHO | ECHONL | ICANON);
        attributes.c_cc[VMIN] = 1;
        attributes.c_cc[VTIME] = 0;
        (void)tcsetattr(STDIN_FILENO, TCSANOW, &attributes);
    }
}

static int configure_wall_clock(void)
{
    static const char marker[] = "aos.wall_time=";
    char command_line[4096];
    int fd = open("/proc/cmdline", O_RDONLY | O_CLOEXEC);
    if (fd < 0)
        return 70;
    ssize_t length;
    do {
        length = read(fd, command_line, sizeof(command_line) - 1);
    } while (length < 0 && errno == EINTR);
    close(fd);
    if (length <= 0)
        return 70;
    command_line[length] = '\0';
    char *encoded = strstr(command_line, marker);
    if (encoded == NULL)
        return 70;
    encoded += sizeof(marker) - 1;
    errno = 0;
    char *end;
    unsigned long long seconds = strtoull(encoded, &end, 10);
    if (errno != 0 || end == encoded ||
        (*end != '\0' && !isspace((unsigned char)*end)) ||
        seconds == 0 || seconds > (unsigned long long)INT64_MAX)
        return 70;
    struct timespec wall_time = {
        .tv_sec = (time_t)seconds,
        .tv_nsec = 0,
    };
    if (clock_settime(CLOCK_REALTIME, &wall_time) < 0) {
        int clock_error = errno;
        struct timeval legacy_wall_time = {
            .tv_sec = (time_t)seconds,
            .tv_usec = 0,
        };

        if (settimeofday(&legacy_wall_time, NULL) < 0) {
            char diagnostic[96];
            int length = snprintf(diagnostic, sizeof(diagnostic),
                                  "AOS CLOCK SET FAILED clock=%d time=%d\n",
                                  clock_error, errno);

            if (length > 0)
                write_all(diagnostic, (size_t)length);
            return 70;
        }
        write_text("AOS CLOCK HOST-ADMITTED settimeofday\n");
        return 0;
    }
    write_text("AOS CLOCK HOST-ADMITTED\n");
    return 0;
}

static int refresh_wall_clock(unsigned long long seconds)
{
    if (seconds == 0 || seconds > (unsigned long long)INT64_MAX)
        return 64;
    struct timespec wall_time = {
        .tv_sec = (time_t)seconds,
        .tv_nsec = 0,
    };
    if (clock_settime(CLOCK_REALTIME, &wall_time) == 0)
        return 0;
    struct timeval legacy_wall_time = {
        .tv_sec = (time_t)seconds,
        .tv_usec = 0,
    };
    return settimeofday(&legacy_wall_time, NULL) == 0 ? 0 : 70;
}

int main(void)
{
    char *line = malloc(MAX_PROTOCOL_LINE_BYTES + 1U);

    if (line == NULL)
        return 70;

    int mount_status = mount_bootstrap_devices();
    configure_console();
    (void)prctl(PR_SET_DUMPABLE, 0, 0, 0, 0);
    umask(0077);
    write_text("AOS LINUX /init\n");
    write_text("AOS BOOTSTRAP buildroot-2026.05.1\n");
    if (mount_status == 0)
        mount_status = attach_immutable_system();
    if (mount_status == 0)
        mount_status = prepare_ephemeral_filesystems();
    if (mount_status == 0 &&
        mount("proc", "/proc", "proc", MS_NOSUID | MS_NODEV | MS_NOEXEC, "") < 0)
        mount_status = 70;
    if (mount_status == 0 &&
        mount("sysfs", "/sys", "sysfs", MS_NOSUID | MS_NODEV | MS_NOEXEC, "") < 0)
        mount_status = 70;
    if (mount_status == 0)
        write_text("AOS SYSTEM dev-2026.07 buildroot-2026.05.1 bash-5.2.37\n");
    if (mount_status == 0)
        mount_status = configure_wall_clock();
    if (mount_status != 0)
        write_text("AOS CLOCK FAILED\n");
    if (mount_status == 0)
        mount_status = prepare_toolchain_runtime();
    if (mount_status == 0)
        mount_status = refresh_home();
    if (mount_status == 0)
        mount_status = refresh_workspace();
    if (mount_status != 0) {
        write_text("AOS STORAGE FAILED\n");
        sync();
        (void)reboot(RB_POWER_OFF);
        return mount_status;
    }

    for (;;) {
        write_text("AOS READY\n");
        ssize_t length = read_protocol_line(line, MAX_PROTOCOL_LINE_BYTES);
        if (length == -2) {
            write_text("AOS PROTOCOL ERROR\n");
            continue;
        }
        if (length <= 0)
            continue;
        line[length] = '\0';
        while (length > 0 && (line[length - 1] == '\n' || line[length - 1] == '\r'))
            line[--length] = '\0';

        size_t prefix_length = strlen(PROTOCOL_PREFIX);
        if ((size_t)length <= prefix_length + TOKEN_HEX_BYTES ||
            memcmp(line, PROTOCOL_PREFIX, prefix_length) != 0 ||
            !valid_token(line + prefix_length)) {
            write_text("AOS PROTOCOL ERROR\n");
            continue;
        }

        char *token = line + prefix_length;
        char *command = token + TOKEN_HEX_BYTES + 1;
        if (*command == '\0') {
            write_text("AOS PROTOCOL ERROR\n");
            continue;
        }

        command_count++;
        emit_begin(token);
        if (strcmp(command, "shutdown") == 0) {
            write_text("shutting down\n");
            emit_end(token, 0);
            sync();
            (void)reboot(RB_POWER_OFF);
            continue;
        }

        int status = execute_command(command);
        emit_end(token, status);
    }
}
