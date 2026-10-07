"""Drive harness-tui in a pty, headless, like a person at a terminal.

Usage: python3 -I cockpit.py <workdir> <rows> <cols> -- <argv...>

- Renders the screen with a small VT interpreter (cursor moves, erases, text;
  SGR colours kept only as a per-cell "colour tag" for yellow/green/red) into
  <workdir>/screen.txt (atomically, whenever it changes).
- Reads commands, one per line, from the FIFO <workdir>/cmd:
    key <Name> [n]     Up Down Left Right Enter Tab BackTab Esc Backspace, or one char
    text <string>      typed as is
    click <col> <row>  1-based SGR mouse press+release
    quit               end the driver (sends nothing)
- Appends every event to <workdir>/log.txt; the raw bytes to <workdir>/raw.bin.
The child's environment is plain (HOME PATH USER LOGNAME SHELL LANG TMPDIR + TERM),
as from a plain Terminal window.
"""
import codecs
import fcntl
import os
import pty
import select
import struct
import sys
import termios
import time
import unicodedata

workdir, rows, cols = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
argv = sys.argv[sys.argv.index("--") + 1:]

KEYS = {
    "Up": b"\x1b[A", "Down": b"\x1b[B", "Right": b"\x1b[C", "Left": b"\x1b[D",
    "Enter": b"\r", "Tab": b"\t", "BackTab": b"\x1b[Z", "Esc": b"\x1b",
    "Backspace": b"\x7f", "PageDown": b"\x1b[6~", "PageUp": b"\x1b[5~",
    "Home": b"\x1b[H", "End": b"\x1b[F", "Space": b" ",
}

env = {k: os.environ[k] for k in ("HOME", "PATH", "USER", "LOGNAME", "SHELL", "LANG", "TMPDIR")
       if k in os.environ}
env["TERM"] = "xterm-256color"

pid, fd = pty.fork()
if pid == 0:
    os.execvpe(argv[0], argv, env)

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

grid = [[" "] * cols for _ in range(rows)]
tags = [[""] * cols for _ in range(rows)]
r = c = 0
fg = ""
pending = ""
decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
COLOURS = {"31": "R", "32": "G", "33": "Y", "91": "R", "92": "G", "93": "Y"}


def width(ch):
    if unicodedata.combining(ch):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


inv = False


def sgr(params):
    global fg, inv
    parts = params.split(";") if params else ["0"]
    i = 0
    while i < len(parts):
        p = parts[i] or "0"
        if p == "0":
            fg = ""
            inv = False
        elif p == "39":
            fg = ""
        elif p == "7":
            inv = True
        elif p == "27":
            inv = False
        elif p in COLOURS:
            fg = COLOURS[p]
        elif p == "38" and i + 2 < len(parts) and parts[i + 1] == "5":
            n = parts[i + 2]
            fg = {"1": "R", "2": "G", "3": "Y", "8": "D", "9": "R", "10": "G", "11": "Y"}.get(n, "")
            i += 2
        elif p == "38" and i + 4 < len(parts) and parts[i + 1] == "2":
            i += 4
            fg = ""
        i += 1


def feed(text):
    """Interpret text; return what is left of an unfinished escape."""
    global r, c, grid, tags
    i, n = 0, len(text)
    while i < n:
        ch = text[i]
        if ch == "\x1b":
            if i + 1 >= n:
                return text[i:]
            nx = text[i + 1]
            if nx == "[":
                j = i + 2
                while j < n and not ("@" <= text[j] <= "~"):
                    j += 1
                if j >= n:
                    return text[i:]
                params, fin = text[i + 2:j], text[j]
                i = j + 1
                if params.startswith("?") or params.startswith(">") or params.startswith("<"):
                    continue
                nums = [int(x) if x.isdigit() else 0 for x in params.split(";")] if params else []

                def num(k, d):
                    return nums[k] if k < len(nums) and nums[k] > 0 else d
                if fin in "Hf":
                    r = min(num(0, 1) - 1, rows - 1)
                    c = min(num(1, 1) - 1, cols - 1)
                elif fin == "A":
                    r = max(r - num(0, 1), 0)
                elif fin == "B":
                    r = min(r + num(0, 1), rows - 1)
                elif fin == "C":
                    c = min(c + num(0, 1), cols - 1)
                elif fin == "D":
                    c = max(c - num(0, 1), 0)
                elif fin == "G":
                    c = min(num(0, 1) - 1, cols - 1)
                elif fin == "J":
                    mode = nums[0] if nums else 0
                    if mode in (2, 3):
                        grid = [[" "] * cols for _ in range(rows)]
                        tags = [[""] * cols for _ in range(rows)]
                    elif mode == 0:
                        for x in range(c, cols):
                            grid[r][x] = " "; tags[r][x] = ""
                        for y in range(r + 1, rows):
                            grid[y] = [" "] * cols; tags[y] = [""] * cols
                elif fin == "K":
                    mode = nums[0] if nums else 0
                    rng = range(c, cols) if mode == 0 else range(0, c + 1) if mode == 1 else range(cols)
                    for x in rng:
                        grid[r][x] = " "; tags[r][x] = ""
                elif fin == "X":
                    for x in range(c, min(c + num(0, 1), cols)):
                        grid[r][x] = " "; tags[r][x] = ""
                elif fin == "m":
                    sgr(params)
                continue
            if nx == "]":
                j = i + 2
                while j < n and text[j] != "\x07" and not (text[j] == "\x1b" and j + 1 < n and text[j + 1] == "\\"):
                    j += 1
                if j >= n:
                    return text[i:]
                i = j + (1 if text[j] == "\x07" else 2)
                continue
            if nx in "()*+":
                if i + 2 >= n:
                    return text[i:]
                i += 3
                continue
            i += 2
            continue
        if ch == "\r":
            c = 0
        elif ch == "\n":
            r = min(r + 1, rows - 1)
        elif ch == "\b":
            c = max(c - 1, 0)
        elif ord(ch) < 32 or ord(ch) == 127:
            pass
        else:
            w = width(ch)
            if w and c < cols:
                grid[r][c] = ch
                tags[r][c] = ("I" if inv else "") + fg
                if w == 2 and c + 1 < cols:
                    grid[r][c + 1] = ""
                c = min(c + w, cols)
        i += 1
    return ""


def screen_text():
    lines = []
    for y in range(rows):
        line = "".join(grid[y]).rstrip()
        # Colour marks: a run of Y/G/R cells noted after the line.
        marks = []
        cur, start = "", 0
        for x in range(cols + 1):
            t = tags[y][x] if x < cols and grid[y][x].strip() else ""
            if x < cols and not grid[y][x].strip() and cur:
                t = cur  # spaces inside a coloured run keep it
            if t != cur:
                if cur:
                    seg = "".join(grid[y][start:x]).strip()
                    if seg:
                        marks.append(f"{cur}:{seg}")
                cur, start = t, x
        lines.append(f"{y + 1:02d}|{line}" + (f"   <<{' | '.join(marks)}>>" if marks else ""))
    return "\n".join(lines) + "\n"


cmd_path = os.path.join(workdir, "cmd")
if not os.path.exists(cmd_path):
    os.mkfifo(cmd_path)
cmd_fd = os.open(cmd_path, os.O_RDWR | os.O_NONBLOCK)
log = open(os.path.join(workdir, "log.txt"), "a", buffering=1)
raw = open(os.path.join(workdir, "raw.bin"), "ab", buffering=0)
log.write(f"{time.strftime('%H:%M:%S')} start {argv}\n")

last_written = ""
dirty = False
last_out = time.time()
last_flush = 0.0
cmd_buf = b""
alive = True
while True:
    try:
        ready, _, _ = select.select([fd, cmd_fd] if alive else [cmd_fd], [], [], 0.25)
    except InterruptedError:
        continue
    if fd in ready:
        try:
            data = os.read(fd, 65536)
        except OSError:
            data = b""
        if not data:
            alive = False
            log.write(f"{time.strftime('%H:%M:%S')} child output closed\n")
            try:
                _, status = os.waitpid(pid, 0)
                log.write(f"{time.strftime('%H:%M:%S')} child exit status {status}\n")
            except ChildProcessError:
                pass
        else:
            raw.write(data)
            pending = feed(pending + decoder.decode(data))
            dirty = True
            last_out = time.time()
    if cmd_fd in ready:
        try:
            cmd_buf += os.read(cmd_fd, 4096)
        except BlockingIOError:
            pass
        while b"\n" in cmd_buf:
            line, cmd_buf = cmd_buf.split(b"\n", 1)
            line = line.decode()
            log.write(f"{time.strftime('%H:%M:%S')} cmd {line!r}\n")
            parts = line.split(" ", 1)
            if parts[0] == "quit":
                sys.exit(0)
            if not alive:
                continue
            if parts[0] == "key":
                name, *rest = parts[1].split()
                times = int(rest[0]) if rest else 1
                seq = KEYS.get(name, name.encode())
                for _ in range(times):
                    os.write(fd, seq)
                    time.sleep(0.08)
            elif parts[0] == "text":
                os.write(fd, parts[1].encode())
            elif parts[0] == "click":
                x, y = parts[1].split()
                os.write(fd, f"\x1b[<0;{x};{y}M".encode())
                time.sleep(0.05)
                os.write(fd, f"\x1b[<0;{x};{y}m".encode())
    if dirty and (time.time() - last_out > 0.15 or time.time() - last_flush > 0.5):
        last_flush = time.time()
        text = screen_text()
        if text != last_written:
            tmp = os.path.join(workdir, ".screen.tmp")
            with open(tmp, "w") as f:
                f.write(f"# {time.strftime('%H:%M:%S')} alive={alive}\n" + text)
            os.replace(tmp, os.path.join(workdir, "screen.txt"))
            last_written = text
        dirty = False
