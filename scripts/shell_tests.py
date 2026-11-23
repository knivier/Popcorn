"""Shell test cases for scripts/shell_test.py.

Each test gets a booted Guest `g`. `run(g, cmd, *must_match, no=(...))` types a
command over COM1, waits for the prompt and regex-checks the output. Tests run
in registration order on one boot (a failure reboots the guest), so state such
as the selected disk carries over: tests that need a disk select it themselves.
"""

from __future__ import annotations

import re
import time
from datetime import datetime, timezone

ERR = r"error|fail|not found|cannot|invalid|unknown|usage|refus|locked"


def register(test, run, expect, forbid):
    # ------------------------------------------------------------------ basics
    @test("basics-help")
    def _(g):
        run(g, "help", r"sysinfo", r"disk", r"dol -help")
        run(g, "halp", r"sysinfo")
        for c in ("mem -help", "cpu -help", "cl -help", "mon -help", "drive -help",
                  "dev -help", "catalog -help", "disk -help", "wrap -help", "dol -help"):
            run(g, c, r"-help|usage|\-")

    @test("basics-bad-input")
    def _(g):
        run(g, "definitelynotacommand", r"unknown|not found|invalid")
        g.cmd("")  # empty line just redraws the prompt
        g.cmd("   ")
        run(g, "mem -bogus", r"unknown|usage|-help|invalid")
        # 200 chars: the line editor caps input at 127 chars; must not crash.
        g.cmd("x" * 200, timeout=40)
        run(g, "help", r"sysinfo")

    @test("basics-wrap-clear")
    def _(g):
        run(g, "wrap -off", r"off|disabled")
        g.cmd("help")
        run(g, "wrap -on", r"on|enabled")
        g.cmd("clear")
        run(g, "help", r"sysinfo")

    @test("basics-tab-complete")
    def _(g):
        g.drain()
        g.buf = ""
        g.send_raw("sysca")
        g.send_raw("\t")
        time.sleep(0.4)
        g.send_raw("\r")
        out = g.wait_prompt()
        expect(out, r"SYSTEM CALL TABLE", cmd="sysca<TAB>")

    # -------------------------------------------------------------------- time
    @test("time-pit-rate")
    def _(g):
        """PIT ticks must track wall time (100 Hz) in poll mode (UEFI) and IRQ mode (GRUB)."""
        def ticks():
            out = run(g, "timer", r"Timer Ticks:\s*\d+", r"Frequency:\s*100")
            return int(re.search(r"Timer Ticks:\s*(\d+)", out).group(1))
        t0, h0 = ticks(), time.monotonic()
        time.sleep(8)
        t1, h1 = ticks(), time.monotonic()
        rate = (t1 - t0) / (h1 - h0)
        if not 80 <= rate <= 120:
            raise AssertionError(f"PIT rate {rate:.1f} ticks/s, expected ~100")
        up = run(g, "uptime", r"Uptime:\s*\d+ ticks", r"Seconds:\s*\d+")
        secs = int(re.search(r"Seconds:\s*(\d+)", up).group(1))
        tk = int(re.search(r"Uptime:\s*(\d+)", up).group(1))
        if abs(secs - tk // 100) > 1:
            raise AssertionError(f"uptime seconds {secs} inconsistent with {tk} ticks")

    @test("time-rtc")
    def _(g):
        out = run(g, "cl -gettime", r"Date:\s*\d{4}-\d\d-\d\d", r"Time:\s*\d\d:\d\d:\d\d")
        d = re.search(r"Date:\s*(\d{4})-(\d\d)-(\d\d)", out)
        t = re.search(r"Time:\s*(\d\d):(\d\d):(\d\d)", out)
        guest = datetime(*map(int, d.groups()), *map(int, t.groups()), tzinfo=timezone.utc)
        skew = abs((datetime.now(timezone.utc) - guest).total_seconds())
        if skew > 120:
            raise AssertionError(f"RTC off by {skew:.0f}s from host UTC: {d.group(0)} {t.group(0)}")

    # ------------------------------------------------------------- system info
    @test("sys-info")
    def _(g):
        si = run(g, "sysinfo", r"Popcorn v\d", r"x86_64", r"Vendor:", r"RAM \(available\):\s*\d+ MB",
                 r"Mode:")
        ram_mb = int(re.search(r"RAM \(available\):\s*(\d+) MB", si).group(1))
        mi = run(g, "mem -info", r"Total:", r"Free:")
        m = re.search(r"Total:\s*([\d.]+)\s*(KiB|MiB|GiB|KB|MB|GB)", mi)
        if not m:
            raise AssertionError(f"mem -info Total has no unit:\n{mi}")
        val = float(m.group(1)) * {"K": 1 / 1024, "M": 1, "G": 1024}[m.group(2)[0]]
        if abs(val - ram_mb) > max(64, ram_mb * 0.05):
            raise AssertionError(f"mem -info Total {m.group(0)} disagrees with sysinfo {ram_mb} MB")
        run(g, "mem -map", r"\d")
        run(g, "mem -use", r"\d")
        run(g, "mem -stats", r"\d")
        run(g, "mem -debug", r"\d")
        run(g, "cpu -info", r"Vendor|vendor")
        hz = run(g, "cpu -hz", r"\d+ MHz")
        mhz = int(re.search(r"(\d+) MHz", hz).group(1))
        if not 100 <= mhz <= 20000:
            raise AssertionError(f"cpu -hz {mhz} MHz out of range")
        run(g, "tasks", r"\d|task", no=(ERR,))
        run(g, "syscalls", r"read", r"write", r"ioctl", r"chdir")
        run(g, "listsys", r"File System|Hierarchy")

    # ------------------------------------------------------------ driver model
    @test("drivers")
    def _(g):
        dl = run(g, "drive -list", r"null", r"zero", r"ttyS0", r"kbd", r"clock")
        names = re.findall(r"(\w+)\(\w[\w-]*\):", dl)
        if len(names) < 6:
            raise AssertionError(f"drive -list parsed {names}")
        forbid(dl, r":fail|:error", cmd="drive -list")
        for n in names:
            run(g, f"drive -info {n}", re.escape(n), no=(r"unknown drive|not found",))
        run(g, "drive -info nosuchdrive", r"unknown|not found|no such")
        run(g, "drive -load null", r"ready|null")
        dv = run(g, "dev -list", r"/dev/null", r"/dev/zero", r"/dev/ttyS0")
        ndev = len(re.findall(r"/dev/\w+", dv))
        cat = run(g, "catalog -list", r"sys:", r"pop:", r"irq:", r"drive:", r"dev:")
        if len(re.findall(r"\bdev:\w+", cat)) != ndev:
            raise AssertionError("catalog dev: entries disagree with dev -list")
        for kind in ("drive", "dev", "pop", "irq", "sys"):
            out = run(g, f"catalog -list {kind}", rf"{kind}:")
            other = {"drive", "dev", "pop", "irq", "sys", "disk"} - {kind}
            forbid(out, *[rf"\b{o}:\w" for o in other], cmd=f"catalog -list {kind}")

    # ------------------------------------------------------------------- tasks
    @test("tasks-lifecycle")
    def _(g):
        run(g, "mon -list", r"PID", r"Idle")
        out = run(g, "mon -debug", r"\d")
        pids = re.findall(r"PID[:\s]*(\d+)", out)
        lst = run(g, "mon -list", r"PID")
        rows = re.findall(r"^(\d+)\s*\|", lst, re.M)
        if len(rows) < 2:
            raise AssertionError(f"mon -debug did not add a task:\n{out}\n{lst}")
        pid = pids[-1] if pids else rows[-1]
        run(g, f"mon -kill {pid}", r"killed")
        run(g, "mon -kill 9999", r"no task|not found")
        run(g, "mon -kill 0", r"idle|cannot|refus|reserved|no task")

    @test("tasks-pool-exhaustion")
    def _(g):
        """Fill the task pool: must fail cleanly, never corrupt memory (old MBI-in-.bss bug)."""
        before = run(g, "mem -map", r"\d")
        failed = False
        for i in range(40):
            out = g.cmd("mon -debug", timeout=20)
            if re.search(ERR + r"|full|no free", out, re.I):
                failed = True
                break
        if not failed:
            raise AssertionError("40 mon -debug calls never reported a full task pool")
        lst = run(g, "mon -list", r"PID")
        run(g, "mon -ultramon", r"kill|\d")
        after_list = run(g, "mon -list", r"Idle")
        if len(re.findall(r"^\d+\s*\|", after_list, re.M)) != 1:
            raise AssertionError(f"mon -ultramon left tasks:\n{after_list}")
        after = run(g, "mem -map", r"\d")
        if before.strip() != after.strip():
            raise AssertionError(f"mem -map changed after task churn:\n--before--\n{before}\n--after--\n{after}")
        # Slots must be reusable after the purge.
        out = run(g, "mon -debug", r"\d", no=(r"full|no free",))
        run(g, "mon -ultramon", r"\S")

    # -------------------------------------------------------------------- disk
    @test("disk-list-and-locks", boots={"uefi"})
    def _(g):
        dl = run(g, "disk -list", r"ram0", r"usb0", r"nvme0.*MASTER-LOCKED")
        # Internal disks may be selected for reads while MASTER-LOCKED; writes must fail.
        run(g, "disk -use nvme0", r"selected")
        run(g, "disk -read 0", r"\S")
        run(g, "disk -write 9 must-not-land", r"LOCKED|refus|fail|denied|read-only")
        run(g, "disk -wipe nvme0 YES", r"LOCKED|refus|fail|denied")
        run(g, "disk -use usb0", r"selected")
        run(g, "disk -use nosuchdisk", r"unknown|fail|LOCKED")
        run(g, "disk -master nvme0", r"YES|confirm")  # without YES: only asks
        dl2 = run(g, "disk -list", r"nvme0.*MASTER-LOCKED")

    @test("disk-raw-rw")
    def _(g):
        run(g, "disk -use ram0", no=(r"fail|LOCKED",))
        run(g, "disk -info", r"selected:\s*ram0", r"writable")
        run(g, "disk -write 7 popcorn-raw-sector-test", no=(ERR,))
        out = run(g, "disk -read 7", r"popcorn-raw-sector-test|70 6f 70 63 6f 72 6e|706f70636f726e")
        run(g, "disk -read 99999999", r"range|fail|error|invalid|beyond")
        run(g, "disk -write", r"usage|-write")
        run(g, "disk -read", r"usage|-read")

    @test("disk-wipe-confirm")
    def _(g):
        run(g, "disk -wipe ram0 YES", r"cannot wipe ram")
        d = data_disk(g)
        run(g, f"disk -wipe {d}", r"YES|confirm")
        run(g, f"disk -use {d}", r"selected")
        run(g, f"disk -wipe {d} YES", r"FAT32 ready|wiped", timeout=120)
        out = run(g, "ls", r"Directory")
        if re.search(r"\.TXT", out, re.I):
            raise AssertionError(f"fresh FAT32 not empty:\n{out}")

    # ---------------------------------------------------------------------- fs
    def data_disk(g):
        """Scratch virtio disk: 'vda' on UEFI (USB boot stick is usb0), 'usb0' on GRUB."""
        disks = run(g, "disk -list", r"ram0")
        return "vda" if re.search(r"\bvda\(", disks) else "usb0"

    def fresh_fs(g, disk):
        run(g, f"disk -use {disk}", no=(r"fail|LOCKED",))
        run(g, f"disk -wipe {disk} YES", r"FAT32 ready|wiped", timeout=120)

    def fs_ops(g):
        run(g, "write a.txt hello world", r"written successfully")
        run(g, "read a.txt", r"hello world")
        run(g, "write a.txt second version", r"written successfully")
        r = run(g, "read a.txt", r"second version")
        forbid(r, r"hello world", cmd="read a.txt after overwrite")
        ls = run(g, "ls", r"A\.TXT")
        run(g, "search a.txt", r"found", r"Location:")
        run(g, "search zz.txt", r"not found")
        run(g, "mkdir d1", r"created successfully")
        run(g, "mkdir d1", r"Failed|exists")
        run(g, "go d1", r"Changed directory")
        g.cmd("ls")
        run(g, "write b.txt in subdir", r"written successfully")
        run(g, "read b.txt", r"in subdir")
        run(g, "back", r"parent")
        run(g, "back", r"Already at root")
        run(g, "read b.txt", r"not found|cannot")
        run(g, "cp a.txt d1", r"copied successfully")
        run(g, "cp a.txt d1", r"Failed|exists")
        run(g, "go d1", r"Changed directory")
        run(g, "read a.txt", r"second version")
        run(g, "back", r"parent")
        run(g, "rm a.txt", r"removed successfully")
        run(g, "read a.txt", r"not found|cannot")
        run(g, "rm a.txt", r"not found|cannot")
        run(g, "delete nothere.txt", r"not found|cannot")
        run(g, "go nodir", r"not found|cannot")
        run(g, "go back", r"Use 'back'")
        run(g, "write", r"usage")
        run(g, "write onlyname", r"usage|empty|format")
        run(g, "read", r"usage")
        # Names that cannot be 8.3 must fail cleanly, not corrupt the directory.
        g.cmd("write averyveryverylongfilename.textfile x")
        g.cmd("write bad/name.txt x")
        out = run(g, "ls", r"Directory", r"D1")
        forbid(out, r"[^\x20-\x7e\n]", cmd="ls after bad names")

    def fs_many_files(g):
        """> one cluster of directory entries: exercises directory extension (#14)."""
        run(g, "mkdir many", r"created")
        run(g, "go many", r"Changed")
        n = 70
        for i in range(n):
            run(g, f"write f{i:02d}.txt content-{i:02d}", r"written successfully")
        ls = run(g, "ls", r"F00\.TXT", r"F69\.TXT")
        names = re.findall(r"F\d\d\.TXT", ls)
        if len(set(names)) != n:
            raise AssertionError(f"ls shows {len(set(names))}/{n} files")
        forbid(ls, r"[^\x20-\x7e\n]", cmd="ls many")
        for i in (0, 33, 69):
            run(g, f"read f{i:02d}.txt", rf"content-{i:02d}")
        for i in range(0, n, 2):
            run(g, f"rm f{i:02d}.txt", r"removed")
        ls = run(g, "ls", r"F01\.TXT")
        if len(re.findall(r"F\d\d\.TXT", ls)) != n // 2:
            raise AssertionError("wrong count after deleting half")
        run(g, "back", r"parent")

    @test("fs-ops-virtio")
    def _(g):
        fresh_fs(g, data_disk(g))
        fs_ops(g)
        fs_many_files(g)

    @test("fs-persist-across-reboot", boots={"uefi"})
    def _(g):
        fresh_fs(g, "vda")
        run(g, "write keep.txt survives-reboot-123", r"written successfully")
        run(g, "mkdir kd", r"created")
        g.stop()
        g.start()
        run(g, "disk -use vda", no=(r"fail|LOCKED",))
        run(g, "read keep.txt", r"survives-reboot-123")
        run(g, "ls", r"KEEP\.TXT", r"KD")

    @test("fs-disk-switch-isolation", boots={"uefi"})
    def _(g):
        """Switching disks must not flush one volume's FAT cache onto another (#13)."""
        fresh_fs(g, "vda")
        run(g, "write v1.txt on-vda", r"written")
        run(g, "disk -master nvme0 YES", r"unlock|armed|master")
        fresh_fs(g, "nvme0")
        run(g, "write r1.txt on-nvme0", r"written")
        run(g, "disk -use vda", no=(r"fail",))
        out = run(g, "ls", r"V1\.TXT")
        forbid(out, r"R1\.TXT", cmd="ls vda")
        run(g, "read v1.txt", r"on-vda")
        run(g, "write v2.txt second-on-vda", r"written")
        run(g, "disk -use nvme0", no=(r"fail",))
        out = run(g, "ls", r"R1\.TXT")
        forbid(out, r"V1\.TXT|V2\.TXT", cmd="ls nvme0")
        run(g, "read r1.txt", r"on-nvme0")
        run(g, "disk -use vda", no=(r"fail",))
        run(g, "read v2.txt", r"second-on-vda")

    # ------------------------------------------------------------------ dolphin
    @test("dolphin-long-file", boots={"uefi"})
    def _(g):
        """Dolphin: > 20 lines and > 1000 bytes must save completely (#48/#51)."""
        fresh_fs(g, "vda")
        g.drain()
        g.buf = ""
        g.send_raw("dol -new note\r")
        time.sleep(1.5)
        lines = [f"line{i:02d} " + "abcdefghij" * 5 for i in range(30)]
        for i, ln in enumerate(lines):
            g.send_raw(ln, pace=0.006)
            if i + 1 < len(lines):
                g.send_raw("\r")
        time.sleep(0.5)
        # Command mode reads the PS/2 queue, so drive Esc :wq through the keyboard.
        g.sendkey("esc")
        time.sleep(0.5)
        g.type_ps2(":wq")
        g.sendkey("ret")
        g.wait_prompt(timeout=30)
        out = run(g, "read note.txt", r"line00", r"line29")
        for i in (0, 15, 29):
            expect(out, rf"line{i:02d} (abcdefghij){{5}}", cmd="read note.txt")
        g.drain()
        g.buf = ""
        g.send_raw("dol -open note\r")
        time.sleep(1.5)
        g.sendkey("esc")
        time.sleep(0.5)
        g.type_ps2(":q")
        g.sendkey("ret")
        g.wait_prompt(timeout=30)

    # ---------------------------------------------------------------- keyboard
    @test("ps2-keyboard-path")
    def _(g):
        """Type a whole command on the emulated PS/2 keyboard (scancode path)."""
        g.drain()
        g.buf = ""
        g.type_ps2("uptime")
        g.sendkey("ret")
        out = g.wait_prompt(timeout=20)
        expect(out, r"Uptime:\s*\d+ ticks", cmd="ps2 uptime")
        # Shift + backspace handling.
        g.drain()
        g.buf = ""
        g.type_ps2("helpX")
        g.sendkey("backspace")
        g.sendkey("ret")
        out = g.wait_prompt(timeout=20)
        expect(out, r"sysinfo", cmd="ps2 help<BS>")

    @test("boot-repeat", boots={"uefi"})
    def _(g):
        """Cold-boot repeatedly: catches intermittent loader/early-boot hangs."""
        slow = []
        for i in range(12):
            g.stop()
            t0 = time.time()
            g.start(boot_timeout=90)
            slow.append(time.time() - t0)
            run(g, "uptime", r"ticks")
        print(f"    boot times: {', '.join(f'{x:.0f}s' for x in slow)}")

    # ------------------------------------------------------------------ stress
    @test("stress-rapid-commands")
    def _(g):
        cmds = ["uptime", "ls", "mem -use", "timer", "disk -info", "dev -list", "mon -list", "help"]
        t0 = time.time()
        for i in range(240):
            c = cmds[i % len(cmds)]
            g.cmd(c, timeout=30)
        up = run(g, "uptime", r"ticks")
        print(f"    240 commands in {time.time() - t0:.1f}s")

    @test("stop-halts")
    def _(g):
        g.drain()
        g.buf = ""
        g.send_raw("stop\r")
        try:
            g.read_until(re.compile(r"safe to power off"), 20, what="Halted")
        except Exception:
            # QEMU may exit on the halt before we read; the text must still be there.
            if "safe to power off" not in g.buf:
                raise
        g.stop()
