"""Eating (python tools/food_test.py): food is eaten only after its eating time, full players
cannot eat normal food, golden apples always work and raise saturation."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, uvarint, svarint, string, read_uvarint

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + b"\x00" + string(""))
    c.pump(0.6)

def give(c, what):
    n = len(c.received)
    command(c, "/give @s " + what)
    c.pump(3.0, lambda: any(p == 0x32 for p, _ in c.received[n:]))

def click_air(c):
    # the server uses its own copy of the held item; same layout as place_block's empty hand
    item = struct.pack("<hH", 0, 0) + uvarint(0) + bytes(1) + uvarint(0) + uvarint(0)
    data = svarint(1) + b"\x01" + svarint(0) * 3 + b"\xff" + svarint(0) + b"\x00" + item
    data += struct.pack("<fff", *c.pos) + struct.pack("<fff", 0, 0, 0) + uvarint(0) + b"\x01\x00"
    c.send(uvarint(0x1e) + svarint(0) + b"\x00" + uvarint(2) + uvarint(0) + data)

def attribute(c, name, since=0):
    """Last value of an attribute the server sent."""
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
            mods, p = read_uvarint(d, p)
            if mods:
                return val  # modifiers are never sent by this server
            if nm == name:
                val = vals[2]
    return val

def slot0_count(c, since):
    for pid, d in reversed(c.received[since:]):
        if pid == 0x31 and d[0] == 0:
            n, p = read_uvarint(d, 1)
            return struct.unpack("<hH", d[p:p + 4])[1]
        if pid == 0x32 and d[:2] == bytes(2):
            i = 2
            _, i = read_uvarint(d, i)  # container name id
            return None
    return None

c = Client("BotFood")
c.join()
c.pump(0.5)
command(c, "/gamemode survival")
command(c, "/clear")

give(c, "bread 1")
n = len(c.received)
click_air(c)
c.pump(2.2)
check(slot0_count(c, n) is None, "bread is not eaten with a full hunger bar")

command(c, "/clear")
give(c, "golden_apple 2")
n = len(c.received)
click_air(c)
c.pump(0.4)
click_air(c)  # "finished" much too early
c.pump(0.3)
check(slot0_count(c, n) is None, "finishing after 0.4 s does not eat")
c.pump(1.6)
check(slot0_count(c, n) == 1, f"the golden apple is eaten after its eating time (left {slot0_count(c, n)})")
sat = attribute(c, "minecraft:player.saturation", n)
check(sat is not None and sat > 10, f"saturation went up ({sat})")

# chorus fruit teleports the player who eats it
command(c, "/clear")
give(c, "chorus_fruit 1")
click_air(c)  # the client's own "done eating" click after the apple
c.pump(0.2)
n = len(c.received)
click_air(c)
c.pump(2.2)
check(any(p == 0x13 for p, _ in c.received[n:]), "eating a chorus fruit teleports (MovePlayer)")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
