#!/usr/bin/env python3
"""Native desktop -> daemon process -> Kanata TCP wire integration tests.

Requires an interactive macOS/Windows desktop; missing focus access is a failure.
The only fake is the Kanata TCP peer. OS notifications, config parsing, daemon,
rule matching, virtual-key ordering and shutdown are all production code.
"""
import argparse
import contextlib
import ctypes
import json
import os
from pathlib import Path
import plistlib
import queue
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

if sys.platform == "win32":
    from ctypes import wintypes
    ctypes.windll.user32.GetForegroundWindow.restype = wintypes.HWND
    ctypes.windll.user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    ctypes.windll.user32.AllowSetForegroundWindow.argtypes = [wintypes.DWORD]

ROOT = Path(__file__).resolve().parents[2]
TIMEOUT = 15


class KanataPeer:
    def __init__(self):
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.listener.settimeout(0.2)
        self.port = self.listener.getsockname()[1]
        self.messages = queue.Queue()
        self.errors = queue.Queue()
        self.stopping = threading.Event()
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def serve(self):
        try:
            while not self.stopping.is_set():
                try:
                    connection, _ = self.listener.accept()
                except socket.timeout:
                    continue
                with connection, connection.makefile("r", encoding="utf-8") as lines:
                    connection.sendall(b'{"LayerChange":{"new":"default"}}\n')
                    for line in lines:
                        message = json.loads(line)
                        if "RequestLayerNames" in message:
                            response = {"LayerNames": {"names": ["default", "browser", "terminal", "vim"]}}
                        elif "RequestFakeKeyNames" in message:
                            response = {"FakeKeyNames": {"names": ["vk_browser", "vk_terminal", "vk_vim"]}}
                        else:
                            self.messages.put(message)
                            # Kanata acknowledges applied layers through LayerChange.
                            if "ChangeLayer" not in message:
                                continue
                            response = {"LayerChange": message["ChangeLayer"]}
                        connection.sendall((json.dumps(response) + "\n").encode())
        except Exception as error:
            if not self.stopping.is_set():
                self.errors.put(error)

    def expect(self, *messages):
        # Assert exact wire order, including releases before layer/press actions.
        for expected in messages:
            try:
                actual = self.messages.get(timeout=TIMEOUT)
            except queue.Empty:
                raise AssertionError(f"No {expected}; peer errors: {list(self.errors.queue)}") from None
            if actual != expected:
                raise AssertionError(f"Expected {expected}; got {actual}")

    def assert_quiet(self):
        try:
            message = self.messages.get(timeout=0.4)
        except queue.Empty:
            return
        raise AssertionError(f"Unexpected extra native action: {message}")

    def close(self):
        self.stopping.set()
        self.listener.close()
        self.thread.join(timeout=2)
        if self.thread.is_alive():
            raise AssertionError("Kanata peer did not stop; daemon left its socket open")
        if not self.errors.empty():
            raise AssertionError(f"Kanata peer failed: {self.errors.get()}")


def layer(name):
    return {"ChangeLayer": {"new": name}}


def vk(name, action):
    return {"ActOnFakeKey": {"name": name, "action": action}}


class App:
    foreground = None

    def __init__(self, executable, log):
        self.process = subprocess.Popen([str(executable)], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=log, text=True)
        self.lines = queue.Queue()
        def read():
            for line in self.process.stdout:
                self.lines.put(line.strip())
        threading.Thread(target=read, daemon=True).start()
        try:
            ready = self.lines.get(timeout=TIMEOUT)
        except queue.Empty:
            self.close()
            raise AssertionError(f"Native fixture did not start: {executable}") from None
        if ready != "READY":
            self.close()
            raise AssertionError(f"Native fixture startup failed: {ready}")

    def send(self, command):
        if sys.platform == "win32" and command == "focus":
            # Only the current foreground process can reliably grant the next app
            # permission. Relay rights through it after the initial runner grant.
            if App.foreground is not None:
                App.foreground.send(f"grant {self.process.pid}")
                App.foreground.expect_reply("GRANTED")
            elif not ctypes.windll.user32.AllowSetForegroundWindow(self.process.pid):
                raise ctypes.WinError()
        self.process.stdin.write(command + "\n")
        self.process.stdin.flush()
        if command == "focus":
            self.expect_reply("FOCUSED")
            App.foreground = self

    def expect_reply(self, expected):
        try:
            reply = self.lines.get(timeout=TIMEOUT)
        except queue.Empty:
            raise AssertionError(f"Native fixture did not acknowledge {expected}") from None
        if reply != expected:
            raise AssertionError(f"Native fixture expected {expected}, received {reply}")

    def close(self):
        if App.foreground is self:
            App.foreground = None
        if self.process.poll() is None:
            with contextlib.suppress(BrokenPipeError, OSError):
                self.send("quit")
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.process.stdin.close()
        self.process.stdout.close()


class NativeFocus(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="kanata-native-")
        cls.addClassCleanup(cls.temp.cleanup)
        cls.work = Path(cls.temp.name)
        cls.executables = []
        if sys.platform == "darwin":
            fixture = cls.work / "window"
            subprocess.run(["swiftc", str(ROOT / "tests/native/window.swift"), "-o", str(fixture)], check=True, timeout=120)
            cls.classes = ["org.kanata.native.a", "org.kanata.native.b", "org.kanata.native.c"]
            for index, bundle_id in enumerate(cls.classes):
                contents = cls.work / f"Fixture{index}.app/Contents"
                (contents / "MacOS").mkdir(parents=True)
                executable = contents / "MacOS/window"
                shutil.copy2(fixture, executable)
                with (contents / "Info.plist").open("wb") as file:
                    plistlib.dump({"CFBundleIdentifier": bundle_id, "CFBundleExecutable": "window",
                                   "CFBundleName": f"Native Fixture {index}", "CFBundlePackageType": "APPL"}, file)
                cls.executables.append(executable)
        elif sys.platform == "win32":
            fixture = cls.work / "window.exe"
            subprocess.run(["cl", "/nologo", "/EHsc", str(ROOT / "tests/native/window.cpp"),
                            f"/Fe:{fixture}", f"/Fo:{cls.work / 'window.obj'}", "user32.lib"], check=True, timeout=120)
            cls.classes = ["native_focus_a", "native_focus_b", "native_focus_c"]
            for letter in "ABC":
                executable = cls.work / f"Native Focus {letter}.exe"
                shutil.copy2(fixture, executable)
                cls.executables.append(executable)
        else:
            raise RuntimeError("Native focus tests require macOS or Windows (no headless fallback)")

    def run_scenario(self):
        # Run twice in the same desktop session to check repeatable process startup.
        with contextlib.ExitStack() as cleanup:
            logs = ROOT / "target/native-test-logs" / f"run-{time.time_ns()}"
            logs.mkdir(parents=True)
            peer = KanataPeer()
            cleanup.callback(peer.close)
            apps = []
            for index, executable in enumerate(self.executables):
                log = cleanup.enter_context((logs / f"fixture-{index}.log").open("w"))
                app = App(executable, log)
                cleanup.callback(app.close)
                apps.append(app)
            # Establish actual focus before starting the daemon (startup snapshot).
            apps[0].send("focus")
            if sys.platform == "win32":
                deadline = time.monotonic() + TIMEOUT
                while time.monotonic() < deadline:
                    pid = ctypes.c_ulong()
                    ctypes.windll.user32.GetWindowThreadProcessId(ctypes.windll.user32.GetForegroundWindow(), ctypes.byref(pid))
                    if pid.value == apps[0].process.pid:
                        break
                    time.sleep(0.05)
                else:
                    self.fail("Windows fixture could not obtain real foreground focus")
            else:
                # NSWorkspace query independently verifies the helper is active.
                query = ('import AppKit; print(NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "")')
                deadline = time.monotonic() + TIMEOUT
                while time.monotonic() < deadline:
                    result = subprocess.run(["swift", "-e", query], capture_output=True, text=True, check=True, timeout=TIMEOUT)
                    if result.stdout.strip() == self.classes[0]:
                        break
                    time.sleep(0.05)
                else:
                    self.fail("macOS fixture could not obtain real foreground focus")
            config = [{"default": "default"}]
            if sys.platform == "win32":
                config.append({"class": f"^{self.classes[0]}$", "title": "^vim", "layer": "vim", "virtual_key": "vk_vim"})
            config.extend([
                {"class": f"^{self.classes[0]}$", "layer": "browser", "virtual_key": "vk_browser"},
                {"class": f"^{self.classes[1]}$", "layer": "terminal", "virtual_key": "vk_terminal"},
            ])
            config_path = logs / "config.json"
            config_path.write_text(json.dumps(config))
            output = cleanup.enter_context((logs / "daemon.log").open("w"))
            flags = subprocess.CREATE_NEW_PROCESS_GROUP if sys.platform == "win32" else 0
            daemon = subprocess.Popen([str(BINARY), "-c", str(config_path), "-p", str(peer.port), "--no-indicator"],
                                      stdout=output, stderr=subprocess.STDOUT, creationflags=flags)
            def kill_daemon():
                if daemon.poll() is None:
                    daemon.kill()
                    daemon.wait(timeout=5)
            cleanup.callback(kill_daemon)
            try:
                peer.expect(layer("browser"), vk("vk_browser", "Press"))
                apps[1].send("focus")
                peer.expect(vk("vk_browser", "Release"), layer("terminal"), vk("vk_terminal", "Press"))
                apps[0].send("focus")
                peer.expect(vk("vk_terminal", "Release"), layer("browser"), vk("vk_browser", "Press"))
                if sys.platform == "win32":
                    # Same HWND, title changes: requires native NAMECHANGE subscription.
                    apps[0].send("vim")
                    peer.expect(vk("vk_browser", "Release"), layer("vim"), vk("vk_vim", "Press"))
                    apps[0].send("title")
                    peer.expect(vk("vk_vim", "Release"), layer("browser"), vk("vk_browser", "Press"))
                apps[2].send("focus")
                peer.expect(vk("vk_browser", "Release"), layer("default"))
                peer.assert_quiet()
                apps[1].send("focus")
                peer.expect(layer("terminal"), vk("vk_terminal", "Press"))
                # Service-style shutdown must release held VKs and restore default.
                daemon.send_signal(signal.CTRL_BREAK_EVENT if sys.platform == "win32" else signal.SIGTERM)
                self.assertEqual(daemon.wait(timeout=TIMEOUT), 0)
                peer.expect(vk("vk_terminal", "Release"), layer("default"))
                peer.assert_quiet()
            except Exception:
                output.flush()
                print(f"Daemon log:\n{(logs / 'daemon.log').read_text()}", file=sys.stderr)
                raise

    def test_native_focus_wire_protocol_and_shutdown(self):
        for iteration in range(2):
            with self.subTest(iteration=iteration):
                self.run_scenario()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    args, remaining = parser.parse_known_args()
    BINARY = args.binary.resolve()
    if not BINARY.is_file():
        parser.error(f"Build the daemon first: {BINARY} is missing")
    unittest.main(argv=[sys.argv[0], *remaining], verbosity=2)
