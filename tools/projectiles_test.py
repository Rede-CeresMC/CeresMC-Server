"""Projectiles (python tools/projectiles_test.py): a snowball pushes the player it hits, a drawn
bow shoots an arrow that hurts."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, svarint, string, read_uvarint

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def give(c, what):
    n = len(c.received)
    command(c, "/give @s " + what)
    c.pump(2.0, lambda: any(p == 0x32 for p, _ in c.received[n:]))

def click_air(c):
    item = struct.pack("<hH", 0, 0) + uvarint(0) + bytes(1) + uvarint(0) + uvarint(0)
    data = svarint(1) + bytes([1]) + svarint(0) * 3 + bytes([255]) + svarint(0) + bytes(1) + item
    data += struct.pack("<fff", *c.pos) + struct.pack("<fff", 0, 0, 0) + uvarint(0) + bytes([1, 0])
    c.send(uvarint(0x1e) + svarint(0) + bytes(1) + uvarint(2) + uvarint(0) + data)

def release(c):
    c.send(uvarint(0x1e) + svarint(0) + bytes(1) + uvarint(4) + uvarint(0))

def health(c, since):
    """Lowest health the server reported (natural regeneration heals quickly afterwards)."""
    val = None
    for pid, d in c.received[since:]:
        if pid != 0x1d:
            continue
        _, p = read_uvarint(d, 0)
        n, p = read_uvarint(d, p)
        for _ in range(n):
            vals = struct.unpack("<6f", d[p:p + 24]); p += 24
            ln, p = read_uvarint(d, p)
            nm = d[p:p + ln].decode(); p += ln
            _, p = read_uvarint(d, p)
            if nm == "minecraft:health":
                val = vals[2] if val is None else min(val, vals[2])
    return val

X, Z = 316.5, 262.5
archer = Client("BotArcher")
archer.join()
target = Client("BotTarget")
target.join()
archer.move(X, 71.62, Z)
target.move(X, 71.62, Z + 4)
archer.pump(0.5); target.pump(0.5)
# a stone floor under both and a clear line between them
command(archer, "/gamemode creative")
STONE = block_id("minecraft:stone")
for dz in range(0, 5):
    bx, bz = int(X), int(Z) + dz
    for y in (70, 71):
        archer.break_block((bx, y, bz))
    archer.place_block((bx, 69, bz), 1, STONE)
    archer.pump(0.15)
archer.pump(0.5)
target.move(X, 71.62, Z + 4)
target.pump(0.5)
command(target, "/gamemode survival")
command(archer, "/gamemode survival")
command(archer, "/clear")
give(archer, "snowball 4")

n = len(target.received)
click_air(archer)
for _ in range(10):
    archer.pump(0.1); target.pump(0.1)
check(any(p == 0x0d and b"snowball" in d for p, d in target.received[n:]), "the other player sees the snowball (AddActor)")
check(any(p == 0x28 for p, _ in target.received[n:]), "the snowball pushes the player it hits")

command(archer, "/clear")
give(archer, "bow 1")
give(archer, "arrow 4")
target.move(X, 71.62, Z + 4)
n = len(target.received)
click_air(archer)
for _ in range(12):
    archer.pump(0.1); target.pump(0.1)
release(archer)
for _ in range(10):
    archer.pump(0.1); target.pump(0.1)
hp = health(target, n)
check(hp is not None and hp <= 15, f"a fully drawn bow shoots an arrow that hurts like vanilla (health {hp})")

archer.close(); target.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
