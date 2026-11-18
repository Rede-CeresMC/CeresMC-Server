"""Mobs (python tools/mobs_test.py): /summon spawns a cow for everyone, it falls to the ground,
wanders, takes hits (with the 2225 ITEM_INTERACT hit too), dies and drops its loot and experience."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, uvarint, svarint, string, read_uvarint

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line, wait=0.8):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(wait)

def zz(v):
    return (v >> 1) ^ -(v & 1)

def mob_adds(c, since, kind):
    out = []
    for p, d in c.received[since:]:
        if p == 0x0d and kind.encode() in d:
            v, _ = read_uvarint(d, 0)
            if zz(v) not in out:  # the bot may get a resent datagram twice
                out.append(zz(v))
    return out

def moves(c, since, eid):
    """y positions sent for that entity"""
    out = []
    for p, d in c.received[since:]:
        if p != 0x12:
            continue
        v, q = read_uvarint(d, 0)
        if v == eid:
            out.append(struct.unpack_from("<fff", d, q + 1))
    return out

def hit(c, eid, action=1):
    pk = uvarint(0x1e) + svarint(0) + bytes(1) + uvarint(3) + uvarint(0)
    pk += uvarint(eid) + uvarint(action) + svarint(0) + bytes(1) + bytes(8) + struct.pack("<fff", *c.pos) + struct.pack("<fff", 0, 0, 0)
    c.send(pk)

def events(c, since, eid):
    out = []
    for p, d in c.received[since:]:
        if p == 0x1b:
            v, q = read_uvarint(d, 0)
            if v == eid:
                out.append(d[q])
    return out

X, Z = 400.5, 300.5

if len(sys.argv) > 1 and sys.argv[1] == "check":
    # after the restart: the pig left by the first run is still there
    c = Client("BotMobs")
    c.join()
    c.move(410.5, 71.62, 310.5)
    c.pump(1.5)
    check(len(mob_adds(c, 0, "minecraft:pig")) >= 1, "mobs survive a restart (mobs.json)")
    print("PASS" if ok else "FAILED")
    sys.exit(0 if ok else 1)

c = Client("BotMobs")
c.join()
other = Client("BotMobs2")
other.join()
c.move(X, 71.62, Z)
other.move(X + 3, 71.62, Z)
c.pump(0.5)

n, n2 = len(c.received), len(other.received)
command(c, f"/summon cow {X} 75 {Z + 2}")
other.pump(1.0)
cows = mob_adds(c, n, "minecraft:cow")
check(len(cows) == 1, "/summon creates a cow")
check(len(mob_adds(other, n2, "minecraft:cow")) == 1, "the other player sees it too")
eid = cows[0] if cows else -1

c.pump(3.0)
ys = [m[1] for m in moves(c, n, eid)]
check(ys and ys[-1] < 72.0, f"it falls to the ground ({ys[-1] if ys else None})")

# a third player joining later sees it as well
late = Client("BotMobs3")
late.join()
late.pump(0.5)
check(len(mob_adds(late, 0, "minecraft:cow")) == 1, "a player who joins later sees it")
late.close()

# hit it: hurt animation, then it dies after enough hits and drops leather/beef + xp
n = len(c.received)
last = moves(c, 0, eid)
if last:
    x, y, z = last[-1]
    c.move(x, y + 1.62, z + 1.0)
    c.pump(0.2)
hit(c, eid)
c.pump(0.4)
check(2 in events(c, n, eid), "a hit plays the hurt animation")
n = len(c.received)
for _ in range(14):
    last = moves(c, 0, eid)
    if last:  # it runs away when hurt: follow it
        x, y, z = last[-1]
        c.move(x, y + 1.62, z + 1.0)
    hit(c, eid, 2)  # the 2225 client sends hits as ITEM_INTERACT
    c.pump(0.6)
    if 3 in events(c, n, eid):
        break
check(3 in events(c, n, eid), "enough hits kill it (death animation)")
c.pump(1.5)
drops = [d for p, d in c.received[n:] if p == 0x0f]
check(len(drops) >= 1, f"it drops its loot ({len(drops)} item entities)")
check(any(p == 0x0d and b"xp_orb" in d for p, d in c.received[n:]), "and experience orbs")
check(any(p == 0x0e and read_uvarint(d, 0)[0] in (eid * 2, eid) for p, d in c.received[n:]), "the body is removed")

# a zombie goes after a survival player and hits them
def eid_of(cl):
    for p, d in cl.received:
        if p == 0x0b:
            v, _ = read_uvarint(d, 0)
            return zz(v)

victim = eid_of(other)
command(other, "/gamemode survival")
other.move(X + 3, 71.62, Z)
other.pump(0.5)
n2 = len(other.received)
command(c, f"/summon zombie {X + 5} 72 {Z}")
other.pump(4.0)
zombies = mob_adds(other, n2, "minecraft:zombie")
check(len(zombies) == 1, "/summon zombie creates a zombie")
check(2 in events(other, n2, victim), "the zombie hits the survival player")
command(other, "/gamemode creative")

n = len(c.received)
command(c, "/summon dragao")
check(any(b"Uso: /summon" in d for p, d in c.received[n:] if p == 0x09), "an unknown mob shows the usage")

# a pig to find again after the restart (run_tests.sh restarts and runs "check")
command(c, "/summon pig 410.5 72 310.5")
print("  waiting for the autosave...")
c.pump(6.0)

print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
