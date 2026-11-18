"""Note blocks and jukeboxes (python tools/music_test.py)."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, string

STONE = block_id("minecraft:stone")
NOTE = block_id("minecraft:noteblock")
JUKEBOX = block_id("minecraft:jukebox")
NOTE_POS, JUKE_POS = (346, 70, 266), (348, 70, 266)

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def sounds(c, since):
    out = []
    for pid, d in c.received[since:]:
        if pid == 0x7b:
            n = d[0]
            out.append(d[1:1 + n].decode(errors="replace"))
    return out

c = Client("BotMusic")
c.join()
c.move(347.5, 71.62, 262.5)
c.pump(0.5)
command(c, "/gamemode creative")
for pos, block in ((NOTE_POS, NOTE), (JUKE_POS, JUKEBOX)):
    c.break_block(pos)
    c.place_block((pos[0], 69, pos[2]), 1, STONE)
    c.pump(0.3)
    c.place_block((pos[0], 69, pos[2]), 1, block)
    c.pump(0.5)
command(c, "/gamemode survival")
command(c, "/clear")

n = len(c.received)
c.place_block(NOTE_POS, 1, 0)
c.pump(1.0)
check("note" in sounds(c, n), f"using a note block plays a note ({sounds(c, n)})")

n0 = len(c.received)
command(c, "/give @s music_disc_cat 1")
c.pump(2.0, lambda: any(p == 0x32 for p, _ in c.received[n0:]))
n = len(c.received)
c.place_block(JUKE_POS, 1, 0)
c.pump(1.0)
check(any(p == 0x56 and b"record.cat" in d for p, d in c.received[n:]), "a disc in the jukebox plays its music (PlaySound with a handle)")
n = len(c.received)
c.place_block(JUKE_POS, 1, 0)
c.pump(1.0)
check(any(p == 0x15c for p, _ in c.received[n:]), "using the jukebox again stops the music (stop by handle)")
check(any(p == 0x0f for p, _ in c.received[n:]), "and ejects the disc")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
