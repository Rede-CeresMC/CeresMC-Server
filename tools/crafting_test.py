"""Crafting test (python tools/crafting_test.py).

Survival: logs into the 2x2 grid -> planks; crafting without ingredients, taking more than was
crafted and a 3x3 recipe without a crafting table are refused; grid items come back on close.
Then a crafting table is placed and clicked, and a chest (3x3) is crafted in it.
"""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, svarint, read_uvarint, string

STONE = block_id("minecraft:stone")
TABLE = block_id("minecraft:crafting_table")
TABLE_POS = (270, 70, 266)
HOTBAR, CRAFT_IN, CREATED = 28, 13, 60

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + b"\x00" + string(""))
    c.pump(0.6)

def read_svarint(d, p):
    v, p = read_uvarint(d, p)
    return (v >> 1) ^ -(v & 1), p

def read_string(d, p):
    n, p = read_uvarint(d, p)
    return d[p:p + n].decode(errors="replace"), p + n

def skip_ingredient(d, p):
    """RecipeIngredient of CraftingData (protocol 2225)."""
    valid, p = read_uvarint(d, p)
    if not valid:
        _, p = read_svarint(d, p)  # aux 0x7fff
        _, p = read_svarint(d, p)  # count
        return p
    kind, p = read_string(d, p)
    if kind == "name":
        _, p = read_string(d, p); _, p = read_svarint(d, p)
    elif kind == "item_tag":
        _, p = read_string(d, p); _, p = read_svarint(d, p)
    elif kind == "molang":
        _, p = read_string(d, p); p += 2
    else:
        raise ValueError("unknown descriptor " + kind)
    _, p = read_svarint(d, p)  # count
    return p

def skip_output(d, p):
    iid, p = read_svarint(d, p)
    if iid == 0:
        return p
    p += 2
    _, p = read_uvarint(d, p)
    _, p = read_svarint(d, p)
    n, p = read_uvarint(d, p)
    return p + n

def skip_tail(d, p, shaped):
    p += 16
    _, p = read_string(d, p)
    _, p = read_svarint(d, p)
    if shaped:
        p += 1
    if d[p]:
        p += 1
        _, p = read_svarint(d, p)
        has = d[p]; p += 1
        if has:
            n, p = read_uvarint(d, p)
            for _ in range(n):
                p = skip_ingredient(d, p)
    else:
        p += 1
    return read_svarint(d, p)

def recipe_ids(c):
    """recipe name -> network id, from the CraftingData packet."""
    data = [d for pid, d in c.received if pid == 0x34]
    if not data:
        return {}
    d, p, out = data[-1], 0, {}
    for shaped in (True, False):
        n, p = read_uvarint(d, p)
        for _ in range(n):
            name, p = read_string(d, p)
            if shaped:
                _, p = read_svarint(d, p); _, p = read_svarint(d, p)
            k, p = read_uvarint(d, p)
            for _ in range(k):
                p = skip_ingredient(d, p)
            k, p = read_uvarint(d, p)
            for _ in range(k):
                p = skip_output(d, p)
            net, p = skip_tail(d, p, shaped)
            out[name] = net
    n, p = read_uvarint(d, p)  # multi recipes: UUID and network id
    for _ in range(n):
        u = d[p:p + 8][::-1] + d[p + 8:p + 16][::-1]
        p += 16
        net, p = read_uvarint(d, p)
        h = u.hex()
        out["multi:" + "-".join([h[:8], h[8:12], h[12:16], h[16:20], h[20:]])] = net
    for _ in range(5):  # user data, chemistry x2, smithing x2
        n, p = read_uvarint(d, p)
        if n:
            raise ValueError("unexpected recipes in an empty bucket")
    n, p = read_uvarint(d, p)  # potion mixes
    for _ in range(n * 6):
        _, p = read_svarint(d, p)
    n, p = read_uvarint(d, p)  # container mixes
    for _ in range(n * 3):
        _, p = read_svarint(d, p)
    n, p = read_uvarint(d, p)  # material reducer
    p += 1  # clean recipes
    if p != len(d):
        raise ValueError(f"CraftingData has {len(d) - p} bytes left over")
    return out

def give(c, what):
    """/give and wait until the item is in the inventory."""
    n = len(c.received)
    command(c, "/give @s " + what)
    c.pump(3.0, lambda: any(p == 0x32 for p, _ in c.received[n:]))

def slot(container, s, stack_id=0):
    return bytes([container]) + b"\x00" + bytes([s]) + struct.pack("<i", stack_id)

req_counter = [-10]
def request(c, actions):
    """Sends one ItemStackRequest and returns True if the server accepted it."""
    req_counter[0] -= 2
    rid = req_counter[0]
    body = svarint(rid) + uvarint(len(actions))
    for ty, data in actions:
        body += uvarint(ty) + b"\x00" + data
    body += uvarint(0) + struct.pack("<i", 0)
    n = len(c.received)
    c.send(uvarint(0x93) + uvarint(1) + body)
    c.pump(3.0, lambda: any(p == 0x94 for p, _ in c.received[n:]))
    resp = [d for p, d in c.received[n:] if p == 0x94]
    return bool(resp) and resp[-1][1] == 0

def craft(net_id, reps=1):
    return (10, svarint(net_id) + bytes([reps]))

def consume(count, container, s):
    return (5, bytes([count]) + slot(container, s))

def take(count, src, dst):
    return (0, bytes([count]) + src + dst)

def main_counts(c):
    """Item counts of the last full inventory (window 0) the server sent."""
    for pid, d in reversed(c.received):
        if pid != 0x31:
            continue
        w, p = read_uvarint(d, 0)
        if w != 0:
            continue
        n, p = read_uvarint(d, p)
        counts = []
        for _ in range(n):
            iid, cnt = struct.unpack("<hH", d[p:p + 4]); p += 4
            _, p = read_uvarint(d, p)
            if d[p]:
                p += 1; _, p = read_uvarint(d, p)
            else:
                p += 1
            _, p = read_uvarint(d, p)
            k, p = read_uvarint(d, p); p += k
            counts.append(cnt if iid else 0)
        return counts
    return None

c = Client("BotCraft")
c.join()
c.move(270.5, 71.62, 262.5)
c.pump(0.5)
ids = recipe_ids(c)
check(len(ids) > 1000, f"CraftingData lists the recipes ({len(ids)})")
planks = ids.get("minecraft:oak_planks")
chest = ids.get("minecraft:Chest_recipeId")
table = ids.get("minecraft:WorkBench_recipeId_from_oak")
check(planks and chest and table, "planks, chest and crafting table recipes are there")

command(c, "/gamemode survival")
command(c, "/clear")
give(c, "oak_log 2")

# 1. log into the 2x2 grid, craft 4 planks into hotbar slot 1
check(request(c, [take(1, slot(HOTBAR, 0), slot(CRAFT_IN, 28))]), "a log can be put into the 2x2 grid")
check(request(c, [craft(planks), consume(1, CRAFT_IN, 28), take(4, slot(CREATED, 50), slot(HOTBAR, 1))]),
      "crafting 4 planks from the log is accepted")
# 2. cheating
check(not request(c, [craft(planks), take(4, slot(CREATED, 50), slot(HOTBAR, 2))]),
      "crafting without using up ingredients is refused")
check(not request(c, [take(1, slot(HOTBAR, 0), slot(CRAFT_IN, 28)), craft(planks), consume(1, CRAFT_IN, 28),
                      take(8, slot(CREATED, 50), slot(HOTBAR, 2))]),
      "taking more than the recipe made is refused")
check(not request(c, [craft(chest), consume(4, HOTBAR, 1), take(1, slot(CREATED, 50), slot(HOTBAR, 2))]),
      "a 3x3 recipe without a crafting table is refused")
check(not request(c, [craft(table), consume(3, HOTBAR, 1), take(1, slot(CREATED, 50), slot(HOTBAR, 2))]),
      "too few ingredients are refused")
# 3. 4 planks -> crafting table
check(request(c, [craft(table), consume(4, HOTBAR, 1), take(1, slot(CREATED, 50), slot(HOTBAR, 2))]),
      "crafting a crafting table from 4 planks is accepted")
# 4. what is left in the grid comes back when the inventory closes
check(request(c, [take(1, slot(HOTBAR, 0), slot(CRAFT_IN, 29))]), "the other log goes into the grid")
n = len(c.received)
c.send(uvarint(0x2f) + bytes([0, 0xff]) + b"\x00")
c.pump(2.0, lambda: any(p == 0x31 for p, _ in c.received[n:]))
counts = main_counts(c)
check(counts is not None and counts[0] == 1, f"closing the inventory gives the grid items back ({counts and counts[:3]})")

# 4b. multi recipe: two wooden pickaxes repaired into one (the server works the result out)
repair = ids.get("multi:00000000-0000-0000-0000-000000000001")
check(repair is not None, "the multi recipes are listed (repair)")
command(c, "/clear")
give(c, "wooden_pickaxe 2")
check(request(c, [take(1, slot(HOTBAR, 0), slot(CRAFT_IN, 28)), take(1, slot(HOTBAR, 1), slot(CRAFT_IN, 29))]),
      "two pickaxes go into the grid")
check(not request(c, [craft(repair), consume(1, CRAFT_IN, 28), take(1, slot(CREATED, 50), slot(HOTBAR, 2))]),
      "a repair using up one pickaxe only is refused")
check(request(c, [craft(repair), consume(1, CRAFT_IN, 28), consume(1, CRAFT_IN, 29), take(1, slot(CREATED, 50), slot(HOTBAR, 2))]),
      "two pickaxes are combined into one (repair recipe)")

# 5. crafting table: place it, click it, craft a chest
command(c, "/gamemode creative")
below = (TABLE_POS[0], TABLE_POS[1] - 1, TABLE_POS[2])
c.break_block(TABLE_POS)
c.place_block(below, 1, STONE)
c.pump(0.5)
c.place_block(below, 1, TABLE)
c.pump(1.0)
n = len(c.received)
c.place_block(TABLE_POS, 1, 0)
c.pump(3.0, lambda: any(p == 0x2e for p, _ in c.received[n:]))
opens = [d for p, d in c.received[n:] if p == 0x2e]
check(opens and opens[-1][1] == 1, "clicking a crafting table opens the workbench window")
command(c, "/clear")
give(c, "oak_planks 8")
check(request(c, [craft(chest), consume(8, HOTBAR, 0), take(1, slot(CREATED, 50), slot(HOTBAR, 1))]),
      "a chest (3x3) can be crafted at the crafting table")

# 6. stonecutter: stone into 2 slabs, only while a stonecutter is open
CUTTER = block_id("minecraft:stonecutter_block")
CUT_POS = (TABLE_POS[0] + 2, TABLE_POS[1], TABLE_POS[2])
slabs = ids.get("minecraft:stonecutter_double_stone_slab")
c.break_block(CUT_POS)
c.place_block((CUT_POS[0], CUT_POS[1] - 1, CUT_POS[2]), 1, STONE)
c.pump(0.4)
c.place_block((CUT_POS[0], CUT_POS[1] - 1, CUT_POS[2]), 1, CUTTER)
c.pump(0.8)
command(c, "/clear")
give(c, "stone 2")
check(not request(c, [craft(slabs), consume(1, HOTBAR, 0), take(2, slot(CREATED, 50), slot(HOTBAR, 1))]),
      "a stonecutter recipe is refused without a stonecutter")
n = len(c.received)
c.place_block(CUT_POS, 1, 0)
c.pump(3.0, lambda: any(p == 0x2e for p, _ in c.received[n:]))
opens = [d for p, d in c.received[n:] if p == 0x2e]
check(opens and opens[-1][1] == 29, "clicking a stonecutter opens its window")
check(request(c, [take(1, slot(HOTBAR, 0), slot(53, 3))]), "stone goes into the stonecutter input")
check(request(c, [craft(slabs), consume(1, 53, 3), take(2, slot(CREATED, 50), slot(HOTBAR, 1))]),
      "the stonecutter cuts stone into two slabs")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
