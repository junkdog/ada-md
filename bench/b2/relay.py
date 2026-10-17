"""relay.py KEYS_FIFO LOG CMD...: run CMD in a pty the size of this terminal, pass its output
through (and to LOG), pass this terminal's input to it (the answers to its queries), and write
whatever arrives on KEYS_FIFO to it as keys. termcmp.zsh runs it in terminals that can't be sent
keys from outside, such as alacritty."""
import fcntl, os, pty, select, sys, termios, tty

fifo, log, cmd = sys.argv[1], sys.argv[2], sys.argv[3:]
size = fcntl.ioctl(0, termios.TIOCGWINSZ, b"\0" * 8)
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, size)
    os.execvp(cmd[0], cmd)
tty.setraw(0)
keys = os.open(fifo, os.O_RDWR)
out = open(log, "wb")
try:
    while True:
        ready, _, _ = select.select([fd, 0, keys], [], [])
        for r in ready:
            data = os.read(r, 65536)
            if r == fd:
                os.write(1, data)
                out.write(data)
            else:
                os.write(fd, data)
except OSError:
    pass
