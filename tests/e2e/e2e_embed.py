"""
Incrustation et pilotage par stdin de flam-player (#201, #119).

    python tests/e2e/e2e_embed.py --build <dossier contenant flam-player.exe>

Cas :
  1. incrustation (Windows) : une fenetre parente de test (classe STATIC) est
     creee, flam-player est lance avec --parent-hwnd <hwnd> --stdin-control ;
     on verifie que sa fenetre devient un enfant du parent appartenant au
     processus du player, qu'elle remplit la zone cliente et en suit le
     redimensionnement ; F6 (envoye a la fenetre du player) rend le focus a la
     fenetre parente et trace "[EMBED] focus-host" (piege clavier, #237) ; la
     ligne "quit" sur stdin l'arrete avec le code 0 ;
  2. parent detruit : la destruction de la fenetre parente arrete le player
     proprement (code 0) ;
  2b. rendu de l'incrustation (Windows, rendu SDL non force) : le player
     trace "[EMBED] rendu : direct3d11". La WebView2 du launcher dessine par
     DirectComposition, au-dessus des rendus direct3d (D3D9), opengl et
     software : avec eux, la zone Lecture reste vide (#332) ;
  3. repli + fin de stdin (sans fenetre, pilote SDL dummy) : un HWND invalide
     donne la trace "[EMBED] repli" et le player tourne quand meme ; la
     fermeture de stdin l'arrete avec le code 0 (voie de sortie de #119).
Rendu logiciel (SDL_RENDER_DRIVER=software) sauf au cas 2b, audio dummy, pas
de capture auto.
Code de sortie : 0 OK, 1 echec, 77 prerequis absents (flam-player, Windows
pour les cas 1 et 2 : seul le cas 3 tourne ailleurs).
"""
import argparse
import os
import subprocess
import sys
import tempfile
import threading
import time

SKIP = 77
IS_WIN = sys.platform == "win32"

SCRIPT = """
local l = lv.label.new(window)
lv.label.set_text(l, "incrustation")
lv.obj.align(l, lv.ALIGN_CENTER, 0, 0)
"""

if IS_WIN:
    import ctypes
    from ctypes import wintypes

    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.CreateWindowExW.restype = wintypes.HWND
    user32.CreateWindowExW.argtypes = [
        wintypes.DWORD, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD,
        ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
        wintypes.HWND, wintypes.HMENU, wintypes.HINSTANCE, wintypes.LPVOID]
    user32.GetWindow.restype = wintypes.HWND
    user32.GetWindow.argtypes = [wintypes.HWND, wintypes.UINT]
    user32.GetWindowThreadProcessId.restype = wintypes.DWORD
    user32.GetWindowThreadProcessId.argtypes = [
        wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
    user32.SetWindowPos.argtypes = [
        wintypes.HWND, wintypes.HWND, ctypes.c_int, ctypes.c_int,
        ctypes.c_int, ctypes.c_int, wintypes.UINT]
    user32.DestroyWindow.argtypes = [wintypes.HWND]
    user32.IsWindow.argtypes = [wintypes.HWND]
    user32.GetWindowLongW.argtypes = [wintypes.HWND, ctypes.c_int]
    user32.PeekMessageW.argtypes = [
        ctypes.POINTER(wintypes.MSG), wintypes.HWND, wintypes.UINT,
        wintypes.UINT, wintypes.UINT]
    user32.TranslateMessage.argtypes = [ctypes.POINTER(wintypes.MSG)]
    user32.DispatchMessageW.argtypes = [ctypes.POINTER(wintypes.MSG)]
    # Meme sensibilite DPI que le launcher (Tauri) : par moniteur v2
    try:
        user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
    except (AttributeError, OSError):
        pass

    WS_OVERLAPPEDWINDOW = 0x00CF0000
    WS_VISIBLE = 0x10000000
    WS_CLIPCHILDREN = 0x02000000
    WS_CHILD = 0x40000000
    GW_CHILD = 5
    GWL_STYLE = -16
    PM_REMOVE = 1
    SWP_NOMOVE = 0x0002
    SWP_NOZORDER = 0x0004
    WM_KEYDOWN = 0x0100
    WM_KEYUP = 0x0101
    VK_F6 = 0x75

    class GUITHREADINFO(ctypes.Structure):
        _fields_ = [("cbSize", wintypes.DWORD), ("flags", wintypes.DWORD),
                    ("hwndActive", wintypes.HWND), ("hwndFocus", wintypes.HWND),
                    ("hwndCapture", wintypes.HWND), ("hwndMenuOwner", wintypes.HWND),
                    ("hwndMoveSize", wintypes.HWND), ("hwndCaret", wintypes.HWND),
                    ("rcCaret", wintypes.RECT)]

    user32.GetGUIThreadInfo.argtypes = [wintypes.DWORD, ctypes.POINTER(GUITHREADINFO)]
    user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM,
                                    wintypes.LPARAM]
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.GetCurrentThreadId.restype = wintypes.DWORD


def focus_window():
    """Fenetre qui a le focus clavier dans la file d'entree de ce thread (parent
    et enfant partagent la meme file : l'enfant est incruste)."""
    info = GUITHREADINFO()
    info.cbSize = ctypes.sizeof(GUITHREADINFO)
    user32.GetGUIThreadInfo(kernel32.GetCurrentThreadId(), ctypes.byref(info))
    return info.hwndFocus


def pump(seconds, until=None):
    """Traite les messages du thread (le parent et l'enfant ont des files
    liees : un parent qui ne pompe pas bloquerait le player). Retourne True
    des que until() est vrai."""
    end = time.monotonic() + seconds
    msg = wintypes.MSG() if IS_WIN else None
    while True:
        if IS_WIN:
            while user32.PeekMessageW(ctypes.byref(msg), None, 0, 0, PM_REMOVE):
                user32.TranslateMessage(ctypes.byref(msg))
                user32.DispatchMessageW(ctypes.byref(msg))
        if until and until():
            return True
        if time.monotonic() >= end:
            return False
        time.sleep(0.02)


def client_size(hwnd):
    rc = wintypes.RECT()
    user32.GetClientRect(hwnd, ctypes.byref(rc))
    return rc.right - rc.left, rc.bottom - rc.top


class Player:
    def __init__(self, exe, args, env_extra):
        env = dict(os.environ)
        env.update({"SDL_AUDIODRIVER": "dummy", "SDL_RENDER_DRIVER": "software",
                    "FLAM_SCREENSHOT_AUTO_MS": "0"})
        env.update(env_extra)
        for key in [k for k, v in env.items() if v is None]:
            del env[key]
        self.proc = subprocess.Popen(
            [exe] + args, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE, env=env, cwd=os.path.dirname(exe))
        self.err = []
        self._t = threading.Thread(target=self._read, daemon=True)
        self._t.start()

    def _read(self):
        for raw in self.proc.stderr:
            self.err.append(raw.decode("utf-8", "replace").rstrip())

    def stderr(self):
        return "\n".join(self.err)

    def has(self, text):
        return any(text in line for line in self.err)

    def exited(self):
        return self.proc.poll() is not None

    def kill(self):
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()


def child_of(parent, pid):
    child = user32.GetWindow(parent, GW_CHILD)
    while child:
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(child, ctypes.byref(owner))
        if owner.value == pid:
            return child
        child = user32.GetWindow(child, 2)  # GW_HWNDNEXT
    return None


def make_parent(w=640, h=480):
    hwnd = user32.CreateWindowExW(
        0, "STATIC", "flam-player : test d'incrustation",
        WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
        100, 100, w, h, None, None, None, None)
    if not hwnd:
        raise OSError(ctypes.get_last_error(), "CreateWindowExW")
    pump(0.2)
    return hwnd


def check(cond, what, player=None):
    if not cond:
        print("[ECHEC] " + what)
        if player:
            print("--- stderr du player ---\n" + player.stderr())
        raise SystemExit(1)
    print("[OK] " + what)


def case_embed(exe, script):
    parent = make_parent()
    p = Player(exe, ["--parent-hwnd", str(parent), "--stdin-control"] + script, {})
    try:
        ok = pump(15, lambda: p.exited() or child_of(parent, p.proc.pid) is not None)
        child = child_of(parent, p.proc.pid)
        check(ok and child and not p.exited(),
              "fenetre du player incrustee dans le parent", p)
        check(pump(10, lambda: p.has("[EMBED] ok")), "trace [EMBED] ok", p)
        style = user32.GetWindowLongW(child, GWL_STYLE) & 0xFFFFFFFF
        check(style & WS_CHILD, "style WS_CHILD", p)
        want = client_size(parent)
        check(pump(5, lambda: client_size(child) == want),
              "l'enfant remplit la zone cliente %s (enfant : %s)"
              % (want, client_size(child)), p)
        user32.SetWindowPos(parent, None, 0, 0, 900, 560, SWP_NOMOVE | SWP_NOZORDER)
        want = client_size(parent)
        check(pump(5, lambda: client_size(child) == want),
              "l'enfant suit le redimensionnement %s (enfant : %s)"
              % (want, client_size(child)), p)
        # Le player a pris le clavier a l'incrustation ; F6 le rend a la fenetre hote.
        check(pump(5, lambda: focus_window() == child),
              "le player incruste a le focus clavier", p)
        user32.PostMessageW(child, WM_KEYDOWN, VK_F6, 0x00400001)
        user32.PostMessageW(child, WM_KEYUP, VK_F6, 0xC0400001)
        check(pump(5, lambda: p.has("[EMBED] focus-host")),
              "F6 : trace [EMBED] focus-host", p)
        check(pump(5, lambda: focus_window() == parent),
              "F6 : le focus est a la fenetre parente (enfant %s, focus %s)"
              % (child, focus_window()), p)
        check(not p.exited(), "le player tourne toujours apres F6", p)
        p.proc.stdin.write(b"quit\n")
        p.proc.stdin.flush()
        check(pump(10, p.exited), "arret apres 'quit' sur stdin", p)
        check(p.proc.returncode == 0, "code de sortie 0 (%s)" % p.proc.returncode, p)
    finally:
        p.kill()
        if user32.IsWindow(parent):
            user32.DestroyWindow(parent)


def case_parent_destroyed(exe, script):
    parent = make_parent()
    p = Player(exe, ["--parent-hwnd", str(parent), "--stdin-control"] + script, {})
    try:
        check(pump(15, lambda: p.exited() or child_of(parent, p.proc.pid) is not None)
              and not p.exited(), "incrustation (cas parent detruit)", p)
        user32.DestroyWindow(parent)
        check(pump(10, p.exited), "arret quand la fenetre hote disparait", p)
        check(p.proc.returncode == 0, "code de sortie 0 (%s)" % p.proc.returncode, p)
        check(p.has("fenetre hote disparue"), "trace de l'arret", p)
    finally:
        p.kill()
        if user32.IsWindow(parent):
            user32.DestroyWindow(parent)


def case_embed_renderer(exe, script):
    """Rendu choisi par le player incruste quand rien ne le force (#332)."""
    parent = make_parent()
    p = Player(exe, ["--parent-hwnd", str(parent), "--stdin-control"] + script,
               {"SDL_RENDER_DRIVER": None})
    try:
        check(pump(15, lambda: p.exited() or p.has("[EMBED] rendu : direct3d11")
                   or p.has("[EMBED] rendu direct3d11 indisponible")),
              "trace du rendu de l'incrustation", p)
        if p.has("[EMBED] rendu direct3d11 indisponible"):
            # Machine sans adaptateur D3D11 : prerequis absent, pas un echec (#335)
            print("[SKIP] cas rendu : direct3d11 indisponible sur cette machine")
            raise SystemExit(SKIP)
        check(p.has("[EMBED] rendu : direct3d11"),
              "rendu direct3d11 (compose au-dessus de la WebView2 du launcher)", p)
        child = child_of(parent, p.proc.pid)
        user32.SetWindowPos(parent, None, 0, 0, 900, 560, SWP_NOMOVE | SWP_NOZORDER)
        want = client_size(parent)
        check(child and pump(5, lambda: client_size(child) == want),
              "direct3d11 : l'enfant suit le redimensionnement %s" % (want,), p)
        pump(1)
        check(not p.exited(), "direct3d11 : le player tourne apres redimensionnement", p)
        p.proc.stdin.write(b"quit\n")
        p.proc.stdin.flush()
        check(pump(10, p.exited), "arret apres 'quit' sur stdin (cas rendu)", p)
        check(p.proc.returncode == 0, "code de sortie 0 (%s)" % p.proc.returncode, p)
    finally:
        p.kill()
        if user32.IsWindow(parent):
            user32.DestroyWindow(parent)


def case_fallback_eof(exe, script):
    p = Player(exe, ["--parent-hwnd", "1", "--stdin-control"] + script,
               {"SDL_VIDEODRIVER": "dummy"})
    try:
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not p.has("[EMBED] repli") and not p.exited():
            time.sleep(0.05)
        check(p.has("[EMBED] repli"), "trace [EMBED] repli sur un HWND invalide", p)
        time.sleep(1.0)
        check(not p.exited(), "le player tourne malgre l'echec d'incrustation", p)
        p.proc.stdin.close()
        try:
            p.proc.wait(10)
        except subprocess.TimeoutExpired:
            pass
        check(p.exited(), "arret a la fin de stdin", p)
        check(p.proc.returncode == 0, "code de sortie 0 (%s)" % p.proc.returncode, p)
    finally:
        p.kill()


def case_no_control(exe, script):
    """Sans --stdin-control, un stdin ferme n'arrete pas le player."""
    p = Player(exe, script, {"SDL_VIDEODRIVER": "dummy"})
    try:
        p.proc.stdin.close()
        time.sleep(2.0)
        check(not p.exited(), "sans --stdin-control, stdin ferme : le player tourne", p)
    finally:
        p.kill()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", required=True, help="dossier contenant flam-player")
    a = ap.parse_args()
    exe = os.path.join(os.path.abspath(a.build),
                       "flam-player.exe" if IS_WIN else "flam-player")
    if not os.path.isfile(exe):
        print("[SKIP] e2e_embed : %s introuvable" % exe)
        return SKIP
    tmp = tempfile.mkdtemp(prefix="flam-embed-")
    path = os.path.join(tmp, "embed.lua")
    with open(path, "w", encoding="utf-8") as f:
        f.write(SCRIPT)
    # dossiers du mode script direct dans le dossier temporaire (sauvegardes)
    script = ["--save-dir", tmp, "--img-dir", tmp, "--sounds-dir", tmp, path]
    case_fallback_eof(exe, script)
    case_no_control(exe, script)
    if not IS_WIN:
        print("[SKIP] incrustation : Windows seulement")
        return 0
    case_embed(exe, script)
    case_parent_destroyed(exe, script)
    case_embed_renderer(exe, script)
    print("e2e_embed : OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
