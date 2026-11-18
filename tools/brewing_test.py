"""Brewing stand (python tools/brewing_test.py): a water bottle, nether wart and blaze powder
start a brew (progress and fuel shown); a stand with nothing to brew stays idle."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, svarint, string, read_uvarint

STONE = block_id("minecraft:stone")
STAND = block_id("minecraft:brewing_stand")
POS = (342, 70, 266)
HOTBAR, INPUT, BOTTLES, FUEL = 28, 9, 10, 11

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

def slot(container, s):
    return bytes([container, 0, s]) + struct.pack("<i", 0)

counter = [-300]
def request(c, actions):
    counter[0] -= 2
    body = svarint(counter[0]) + uvarint(len(actions))
    for ty, data in actions:
        body += uvarint(ty) + bytes(1) + data
    body += uvarint(0) + struct.pack("<i", 0)
    n = len(c.received)
    c.send(uvarint(0x93) + uvarint(1) + body)
    c.pump(3.0, lambda: any(p == 0x94 for p, _ in c.received[n:]))
    resp = [d for p, d in c.received[n:] if p == 0x94]
    return bool(resp) and resp[-1][1] == 0

def take(count, src, dst):
    return (0, bytes([count]) + src + dst)

def set_data(c, since):
    """[(property, value)] of ContainerSetData packets."""
    out = []
    for pid, d in c.received[since:]:
        if pid == 0x33:
            p = 1
            prop, p = read_uvarint(d, p)
            val, p = read_uvarint(d, p)
            out.append(((prop >> 1) ^ -(prop & 1), (val >> 1) ^ -(val & 1)))
    return out

c = Client("BotBrew")
c.join()
c.move(POS[0] + 0.5, 71.62, POS[2] - 2.5)
c.pump(0.5)
command(c, "/gamemode creative")
c.break_block(POS)
c.place_block((POS[0], 69, POS[2]), 1, STONE)
c.pump(0.4)
c.place_block((POS[0], 69, POS[2]), 1, STAND)
c.pump(0.8)
command(c, "/gamemode survival")
command(c, "/clear")
give(c, "potion 1 0")
give(c, "nether_wart 1")
give(c, "blaze_powder 1")

n = len(c.received)
c.place_block(POS, 1, 0)
c.pump(3.0, lambda: any(p == 0x2e for p, _ in c.received[n:]))
opens = [d for p, d in c.received[n:] if p == 0x2e]
check(opens and opens[-1][1] == 4, "clicking a brewing stand opens its window")
check(request(c, [take(1, slot(HOTBAR, 0), slot(BOTTLES, 1))]), "a water bottle goes into a bottle slot")
check(request(c, [take(1, slot(HOTBAR, 2), slot(FUEL, 4))]), "blaze powder goes into the fuel slot")
n = len(c.received)
check(request(c, [take(1, slot(HOTBAR, 1), slot(INPUT, 0))]), "nether wart goes into the reagent slot")
c.pump(2.0)
data = set_data(c, n)
check(any(p == 0 and 0 < v < 400 for p, v in data), f"the brew starts (progress counting down)")
check(any(p == 1 and v == 19 for p, v in data), "the blaze powder became 20 brews of fuel (one used)")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
