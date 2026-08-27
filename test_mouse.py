import ctypes
import time

user32 = ctypes.windll.user32

INPUT_MOUSE = 0
MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_MOVE_NOCOALESCE = 0x2000


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", ctypes.c_long),
        ("dy", ctypes.c_long),
        ("mouseData", ctypes.c_ulong),
        ("dwFlags", ctypes.c_ulong),
        ("time", ctypes.c_ulong),
        ("dwExtraInfo", ctypes.c_size_t),
    ]


class INPUT_UNION(ctypes.Union):
    _fields_ = [
        ("mi", MOUSEINPUT),
    ]


class INPUT(ctypes.Structure):
    _anonymous_ = ("u",)

    _fields_ = [
        ("type", ctypes.c_ulong),
        ("u", INPUT_UNION),
    ]


def move_mouse(dx, dy):
    mouse_input = INPUT(
        type=INPUT_MOUSE,
        mi=MOUSEINPUT(
            dx=dx,
            dy=dy,
            mouseData=0,
            dwFlags=MOUSEEVENTF_MOVE | MOUSEEVENTF_MOVE_NOCOALESCE,
            time=0,
            dwExtraInfo=0,
        ),
    )

    result = user32.SendInput(
        1,
        ctypes.byref(mouse_input),
        ctypes.sizeof(INPUT),
    )

    print("SendInput:", result)


print("Retourne dans Deadlock : test dans 5 secondes...")
time.sleep(5)

print("Injection...")

for _ in range(20):
    move_mouse(0, 25)
    time.sleep(0.01)

print("Terminé.")