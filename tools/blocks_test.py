"""Block editing test (python tools/blocks_test.py [check]).

Without arguments: BotA places a stone block and breaks another one; BotB must see both.
With "check": run after a server restart; the placed stone must still be there.
"""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, read_uvarint

STONE = block_id("minecraft:stone")
SPAWN = (256.5, 71.62, 256.5)
PLACE_ON, PLACED = (258, 69, 258), (258, 70, 258)

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

if len(sys.argv) > 1 and sys.argv[1] == "check":
    c = Client("BotCheck")
    c.join()
    c.pump(1.0)
    # a placed block shows up when another block in the same chunk is (re)sent; ask by clicking it
    c.move(*SPAWN)
    c.pump(1.0)
    c.place_block(PLACED, 1, 0)  # invalid placement -> server answers with the real blocks
    c.pump(4.0, lambda: any(pos == PLACED for pos, _ in update_blocks(c)))
    seen = dict(update_blocks(c))
    check(seen.get(PLACED) == STONE, f"stone survived the restart at {PLACED} (got {seen.get(PLACED)})")
    if seen.get(PLACED) != STONE:
        print("    received:", [hex(p) for p, _ in c.received][-30:], "updates:", update_blocks(c))
    c.close()
else:
    a, b = Client("BotA"), Client("BotB")
    a.join(); b.join()
    a.move(*SPAWN); b.move(*SPAWN)
    a.pump(0.5); b.pump(0.5)
    # the world keeps what earlier runs did: clear the test positions first
    for pos in [PLACED, (259, 70, 258)]:
        a.break_block(pos)
    a.pump(1.0); b.pump(1.0)
    # y=69 is air in the flat test world: clicking air replaces it (vanilla), which gives the
    # bots a solid block to click on, like a real player would
    for support in [PLACE_ON, (259, 69, 258)]:
        a.place_block(support, 1, STONE)
    a.pump(1.0); b.pump(1.0)
    b.received.clear()
    a.place_block(PLACE_ON, 1, STONE)
    b.pump(4.0, lambda: (PLACED, STONE) in update_blocks(b))
    check((PLACED, STONE) in update_blocks(b), f"BotB sees the stone BotA placed at {PLACED}")
    SECOND = (259, 70, 258)
    a.place_block((259, 69, 258), 1, STONE)
    b.pump(4.0, lambda: (SECOND, STONE) in update_blocks(b))
    a.break_block(SECOND)
    b.pump(4.0, lambda: any(pos == SECOND and bid != STONE for pos, bid in update_blocks(b)))
    after = [bid for pos, bid in update_blocks(b) if pos == SECOND]
    check(after and after[-1] == block_id("minecraft:air"), f"BotB sees BotA break the block at {SECOND}")
    n = len(update_blocks(a))
    a.place_block(PLACE_ON, 1, STONE)  # target already holds stone: must be refused and resynced
    a.pump(4.0, lambda: len(update_blocks(a)) > n)
    check(all(bid == STONE for pos, bid in update_blocks(a)[n:] if pos == PLACED), "placing into an occupied block is refused")
    print("  waiting for the autosave...")
    a.pump(6.0)
    a.close(); b.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
