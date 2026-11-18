"""Beds (python tools/sleep_test.py): at night a bed sets the respawn point and the player lies
down; after 5 s of everybody sleeping the night is skipped."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, string, read_uvarint

STONE = block_id("minecraft:stone")
BED = block_id("minecraft:bed")
POS = (320, 70, 266)

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def texts(c, since):
    return [d for p, d in c.received[since:] if p == 0x09]

c = Client("BotSleep")
c.join()
c.move(POS[0] + 0.5, 71.62, POS[2] - 2.5)
c.pump(0.5)
command(c, "/gamemode creative")
head = (POS[0], POS[1], POS[2] + 1)
for p in (POS, head):
    c.break_block(p)
    c.place_block((p[0], p[1] - 1, p[2]), 1, STONE)
c.pump(0.4)
c.place_block((POS[0], POS[1] - 1, POS[2]), 1, BED)
c.pump(0.8)
command(c, "/time set day")

n = len(c.received)
c.place_block(POS, 1, 0)
c.pump(1.0)
check(any("renascimento".encode() in d for d in texts(c, n)), "a bed sets the respawn point")
check(any("noite".encode() in d for d in texts(c, n)), "nobody sleeps during the day")

command(c, "/time set night")
n = len(c.received)
c.place_block(POS, 1, 0)
c.pump(1.0)
check(any(p == 0x27 for p, _ in c.received[n:]), "at night the player lies down (SetActorData)")
# wait for the wake-up itself (the /time command's own SetTime may arrive late)
c.pump(8.0, lambda: any(p == 0x2c for p, _ in c.received[n:]))
times = [struct.unpack("<i", bytes(4))[0] for p, d in c.received[n:] if p == 0x0a]
check(bool(times), "after 5 s asleep the night is skipped (SetTime)")
check(any(p == 0x2c for p, _ in c.received[n:]), "the player wakes up (Animate)")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
