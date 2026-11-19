"""Mobs (python tools/mobs_test.py): /summon spawns a cow for everyone, it falls to the ground,
wanders, takes hits (with the 2225 ITEM_INTERACT hit too), dies and drops its loot and experience."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, svarint, string, read_uvarint

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

def both(a, b, seconds):
    """pump two bots in turns so neither times out"""
    for _ in range(int(seconds / 0.5)):
        a.pump(0.4)
        b.pump(0.1)

X, Z = 3000.5, 3000.5  # far from anything built in a copied world
G = 4  # ground of the flat test world (feet height)
EYE = G + 1.62

if len(sys.argv) > 1 and sys.argv[1] == "check":
    # after the restart: the pig left by the first run is still there
    c = Client("BotMobs")
    c.join()
    command(c, f"/tp @s {X + 10} {G} {Z + 10}")
    c.move(X + 10, EYE, Z + 10)
    c.pump(1.5)
    check(len(mob_adds(c, 0, "minecraft:pig")) >= 1, "mobs survive a restart (mobs.json)")
    print("PASS" if ok else "FAILED")
    sys.exit(0 if ok else 1)

c = Client("BotMobs")
c.join()
other = Client("BotMobs2")
other.join()
command(c, f"/tp @s {X} {G} {Z}")
command(other, f"/tp @s {X + 3} {G} {Z}")
c.move(X, EYE, Z)
other.move(X + 3, EYE, Z)
c.pump(0.5)

# a spawn egg used on the ground makes its mob (the second bot's hotbar slot 0 holds it)
other.move(X + 120, EYE, Z)
other.pump(0.3)
command(other, "/give @s pig_spawn_egg 1")
other.pump(0.5)
n2 = len(other.received)
other.place_block((int(X + 120), G - 1, int(Z + 3)), 1, 0)
both(other, c, 1.5)
check(len(mob_adds(other, n2, "minecraft:pig")) == 1, "a spawn egg used on the ground creates the mob")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# a spawn egg on a spawner sets its mob; with a player near it spawns them
other.move(X + 140, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s mob_spawner 1")
other.place_block((int(X + 140), G - 1, int(Z + 4)), 1, block_id("minecraft:mob_spawner"))
other.pump(0.5)
command(other, "/clear")
command(other, "/give @s cow_spawn_egg 1")
n2 = len(other.received)
other.place_block((int(X + 140), G, int(Z + 4)), 1, 0)
both(other, c, 3.0)
check(any(p == 0x38 and b"minecraft:cow" in d for p, d in other.received[n2:]), "a spawn egg sets the spawner's mob")
check(len(mob_adds(other, n2, "minecraft:cow")) >= 1, "the spawner spawns that mob near a player")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# bone meal on a grass block sprouts grass and flowers around
other.move(X + 160, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s bone_meal 4")
n2 = len(other.received)
other.place_block((int(X + 160), G - 1, int(Z + 3)), 1, 0)
both(other, c, 1.0)
check(sum(1 for p, _ in other.received[n2:] if p == 0x15) >= 2, "bone meal on grass makes plants grow around")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# an oak sapling given bone meal grows into a tree
other.move(X + 180, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s oak_sapling 1")
other.place_block((int(X + 180), G - 1, int(Z + 4)), 1, block_id("minecraft:oak_sapling"))
other.pump(0.5)
command(other, "/clear")
command(other, "/give @s bone_meal 64")
n2 = len(other.received)
for _ in range(12):
    other.place_block((int(X + 180), G, int(Z + 4)), 1, 0)
    other.pump(0.25)
both(other, c, 1.0)
updates = sum(1 for p, _ in other.received[n2:] if p == 0x15)
check(updates >= 20, f"bone meal grows a sapling into a tree ({updates} block updates)")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# an armor stand is placed with its item and wears what it is given
other.move(X + 200, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s armor_stand 1")
n2 = len(other.received)
other.place_block((int(X + 200), G - 1, int(Z + 3)), 1, 0)
both(other, c, 1.0)
stands = mob_adds(other, n2, "minecraft:armor_stand")
check(len(stands) == 1, "an armor stand item places an armor stand")
command(other, "/clear")
command(other, "/give @s iron_helmet 1")
n2 = len(other.received)
if stands:
    hit(other, stands[0], 0)
both(other, c, 1.5)
check(stands and any(p == 0x20 and read_uvarint(d, 0)[0] == stands[0] for p, d in other.received[n2:]), "the armor stand wears the helmet (MobArmorEquipment)")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# melon seeds planted on farmland made with a hoe
other.move(X + 220, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s wooden_hoe 1")
other.place_block((int(X + 220), G - 1, int(Z + 3)), 1, 0)
other.pump(0.4)
command(other, "/clear")
command(other, "/give @s melon_seeds 1")
n2 = len(other.received)
other.place_block((int(X + 220), G - 1, int(Z + 3)), 1, 0)
both(other, c, 0.8)
check(any(p == 0x15 for p, _ in other.received[n2:]), "melon seeds are planted on farmland")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# lightning on a pig turns it into a zombified piglin (the pack's damage sensor)
other.move(X + 240, EYE, Z)
other.pump(0.3)
n2 = len(other.received)
command(other, f"/summon pig {X + 240} {G} {Z + 6} spawn_adult")
other.pump(0.5)
command(other, f"/summon lightning_bolt {X + 240} {G} {Z + 6}")
both(other, c, 1.5)
check(any(p == 0x0d and b"lightning_bolt" in d for p, d in other.received[n2:]), "/summon lightning_bolt strikes")
check(len(mob_adds(other, n2, "minecraft:zombie_pigman")) >= 1, "lightning turns a pig into a zombified piglin")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# twelve eyes of ender in a ring of end portal frames open the portal
other.move(X + 260, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s end_portal_frame 1")
cx, cz = int(X + 260), int(Z + 6)
ring = []
for d in (-1, 0, 1):
    ring += [(cx + d, cz - 2), (cx + d, cz + 2), (cx - 2, cz + d), (cx + 2, cz + d)]
for (rx, rz) in ring:
    other.place_block((rx, G - 1, rz), 1, block_id("minecraft:end_portal_frame"))
    other.pump(0.1)
other.pump(0.5)
command(other, "/clear")
command(other, "/give @s ender_eye 12")
n2 = len(other.received)
for (rx, rz) in ring:
    other.place_block((rx, G, rz), 1, 0)
    other.pump(0.1)
both(other, c, 1.0)
updates = sum(1 for p, _ in other.received[n2:] if p == 0x15)
check(updates >= 21, f"twelve eyes of ender open the end portal ({updates} block updates)")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# candles stack in one block and flint and steel lights them
other.move(X + 280, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s candle 4")
other.place_block((int(X + 280), G - 1, int(Z + 3)), 1, block_id("minecraft:candle"))
other.pump(0.3)
n2 = len(other.received)
other.place_block((int(X + 280), G, int(Z + 3)), 1, block_id("minecraft:candle"))
both(other, c, 0.6)
check(any(p == 0x15 for p, _ in other.received[n2:]), "a candle used on a candle adds a second one")
command(other, "/clear")
command(other, "/give @s flint_and_steel 1")
n2 = len(other.received)
other.place_block((int(X + 280), G, int(Z + 3)), 1, 0)
both(other, c, 0.6)
check(any(p == 0x15 for p, _ in other.received[n2:]), "flint and steel lights the candles")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# four dark oak saplings in a square grow into a dark oak
other.move(X + 300, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s dark_oak_sapling 4")
sq = [(0, 0), (1, 0), (0, 1), (1, 1)]
for (dx, dz) in sq:
    other.place_block((int(X + 300) + dx, G - 1, int(Z + 5) + dz), 1, block_id("minecraft:dark_oak_sapling"))
    other.pump(0.15)
command(other, "/clear")
command(other, "/give @s bone_meal 64")
n2 = len(other.received)
for _ in range(12):
    other.place_block((int(X + 300), G, int(Z + 5)), 1, 0)
    other.pump(0.25)
both(other, c, 1.0)
updates = sum(1 for p, _ in other.received[n2:] if p == 0x15)
check(updates >= 40, f"four dark oak saplings in a square grow a dark oak ({updates} block updates)")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# four spruce saplings in a square grow a giant spruce
other.move(X + 330, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s spruce_sapling 4")
for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)]:
    other.place_block((int(X + 330) + dx, G - 1, int(Z + 6) + dz), 1, block_id("minecraft:spruce_sapling"))
    other.pump(0.15)
command(other, "/clear")
command(other, "/give @s bone_meal 64")
n2 = len(other.received)
for _ in range(12):
    other.place_block((int(X + 330), G, int(Z + 6)), 1, 0)
    other.pump(0.25)
both(other, c, 1.0)
updates = sum(1 for p, _ in other.received[n2:] if p == 0x15)
check(updates >= 80, f"four spruce saplings in a square grow a giant spruce ({updates} block updates)")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# an azalea given bone meal grows into an azalea tree
other.move(X + 350, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s azalea 1")
other.place_block((int(X + 350), G - 1, int(Z + 4)), 1, block_id("minecraft:azalea"))
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s bone_meal 64")
n2 = len(other.received)
for _ in range(12):
    other.place_block((int(X + 350), G, int(Z + 4)), 1, 0)
    other.pump(0.25)
both(other, c, 1.0)
updates = sum(1 for p, _ in other.received[n2:] if p == 0x15)
check(updates >= 15, f"bone meal grows an azalea into a tree ({updates} block updates)")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# a mangrove propagule given bone meal grows through its stages into a mangrove
other.move(X + 370, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s mangrove_propagule 1")
other.place_block((int(X + 370), G - 1, int(Z + 4)), 1, block_id("minecraft:mangrove_propagule"))
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s bone_meal 64")
n2 = len(other.received)
for _ in range(8):
    other.place_block((int(X + 370), G, int(Z + 4)), 1, 0)
    other.pump(0.25)
both(other, c, 1.0)
updates = sum(1 for p, _ in other.received[n2:] if p == 0x15)
check(updates >= 40, f"bone meal grows a mangrove propagule into a mangrove ({updates} block updates)")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# bone meal on a mushroom makes a huge one, on a moss block spreads moss
other.move(X + 390, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s brown_mushroom 1")
other.place_block((int(X + 390), G - 1, int(Z + 4)), 1, block_id("minecraft:brown_mushroom"))
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s moss_block 1")
other.place_block((int(X + 400), G - 1, int(Z + 4)), 1, block_id("minecraft:moss_block"))
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s bone_meal 64")
n2 = len(other.received)
for _ in range(10):
    other.place_block((int(X + 390), G, int(Z + 4)), 1, 0)
    other.pump(0.25)
both(other, c, 1.0)
updates = sum(1 for p, _ in other.received[n2:] if p == 0x15)
check(updates >= 30, f"bone meal grows a huge mushroom ({updates} block updates)")
n2 = len(other.received)
other.place_block((int(X + 400), G, int(Z + 4)), 1, 0)
both(other, c, 1.0)
updates = sum(1 for p, _ in other.received[n2:] if p == 0x15)
check(updates >= 10, f"bone meal on a moss block spreads moss ({updates} block updates)")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# a sweet berry bush grown with bone meal is picked by hand
other.move(X + 410, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s sweet_berries 1")
other.place_block((int(X + 410), G - 1, int(Z + 4)), 1, 0)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s bone_meal 3")
for _ in range(3):
    other.place_block((int(X + 410), G, int(Z + 4)), 1, 0)
    other.pump(0.25)
command(other, "/clear")
n2 = len(other.received)
other.place_block((int(X + 410), G, int(Z + 4)), 1, 0)
both(other, c, 1.0)
check(any(p == 0x0f for p, _ in other.received[n2:]), "ripe sweet berries are picked from the bush")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# a slab placed in water keeps the water (second block layer)
other.move(X + 430, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s water_bucket 1")
other.place_block((int(X + 430), G - 1, int(Z + 3)), 1, 0)
other.pump(0.5)
command(other, "/clear")
command(other, "/give @s oak_slab 1")
n2 = len(other.received)
other.place_block((int(X + 430), G - 1, int(Z + 3)), 1, block_id("minecraft:oak_slab"))
both(other, c, 0.8)
def layer_of(d):
    # UpdateBlock: block pos (svarint x, uvarint y, svarint z), id, flags, layer
    p = 0
    for _ in range(3):
        _, p = read_uvarint(d, p)
    for _ in range(2):
        _, p = read_uvarint(d, p)
    return read_uvarint(d, p)[0]
check(any(p == 0x15 and layer_of(d) == 1 for p, d in other.received[n2:]), "a slab placed in water is waterlogged (layer 1)")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# a water bucket used on a dry slab waterlogs it
other.move(X + 440, EYE, Z)
other.pump(0.3)
command(other, "/clear")
command(other, "/give @s oak_slab 1")
other.place_block((int(X + 440), G - 1, int(Z + 3)), 1, block_id("minecraft:oak_slab"))
other.pump(0.4)
command(other, "/clear")
command(other, "/give @s water_bucket 1")
n2 = len(other.received)
other.place_block((int(X + 440), G, int(Z + 3)), 1, 0)
both(other, c, 0.8)
check(any(p == 0x15 and layer_of(d) == 1 for p, d in other.received[n2:]), "a water bucket waterlogs a slab")
both(other, c, 1.5)
check(sum(1 for p, d in other.received[n2:] if p == 0x15 and layer_of(d) == 0) >= 3, "water flows out of a waterlogged slab")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# a saddled pig can be ridden (SetActorLink) and left (Interact StopRiding)
other.move(X + 450, EYE, Z)
other.pump(0.3)
n2 = len(other.received)
command(other, f"/summon pig {X + 450} {G} {Z + 2} spawn_adult")
other.pump(0.5)
pigs = mob_adds(other, n2, "minecraft:pig")
command(other, "/clear")
command(other, "/give @s saddle 1")
if pigs:
    hit(other, pigs[0], 0)
    other.pump(0.5)
command(other, "/clear")
other.pump(0.5)
n2 = len(other.received)
if pigs:
    hit(other, pigs[0], 0)
both(other, c, 1.0)
links = [d for p, d in other.received[n2:] if p == 0x29]
check(len(links) >= 1, "a saddled pig can be ridden (SetActorLink)")
n2 = len(other.received)
if pigs:
    other.send(uvarint(0x21) + bytes([3]) + uvarint(pigs[0]))
both(other, c, 0.8)
check(any(p == 0x29 for p, _ in other.received[n2:]), "StopRiding gets the player off")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# a lead ties a cow to the player (its metadata changes: LeashHolder)
other.move(X + 470, EYE, Z)
other.pump(0.3)
n2 = len(other.received)
command(other, f"/summon cow {X + 470} {G} {Z + 2} spawn_adult")
other.pump(0.5)
cows2 = mob_adds(other, n2, "minecraft:cow")
command(other, "/clear")
command(other, "/give @s lead 1")
other.pump(0.5)
n2 = len(other.received)
if cows2:
    hit(other, cows2[0], 0)
both(other, c, 1.0)
check(cows2 and any(p == 0x27 and read_uvarint(d, 0)[0] == cows2[0] for p, d in other.received[n2:]), "a lead ties a cow to the player")
command(other, "/clear")
other.move(X + 3, EYE, Z)
other.pump(0.3)

# two cows fed with wheat make a baby
c.move(X + 80, EYE, Z)
c.pump(0.3)
command(c, "/give @s wheat 10")
c.pump(0.5)
n = len(c.received)
command(c, f"/summon cow {X + 80} {G} {Z + 1.5} spawn_adult")
command(c, f"/summon cow {X + 81} {G} {Z + 1.5} spawn_adult")
c.pump(1.0)
cows = mob_adds(c, n, "minecraft:cow")
for e in cows:
    last = moves(c, 0, e)
    if last:
        x, y, z = last[-1]
        c.move(x, y + 1.62, z - 1.0)
        c.pump(0.2)
    hit(c, e, 0)  # use the wheat on it
    c.pump(0.3)
for _ in range(8):
    c.pump(0.8)
    other.pump(0.2)
check(len(mob_adds(c, n, "minecraft:cow")) >= 3, f"fed cows breed ({len(mob_adds(c, n, 'minecraft:cow'))} cows)")

c.move(X, EYE, Z)
c.pump(0.3)
n, n2 = len(c.received), len(other.received)
command(c, f"/summon cow {X} {G + 3} {Z + 2} spawn_adult")  # an adult: babies drop nothing
other.pump(1.0)
cows = mob_adds(c, n, "minecraft:cow")
check(len(cows) == 1, "/summon creates a cow")
check(len(mob_adds(other, n2, "minecraft:cow")) == 1, "the other player sees it too")
eid = cows[0] if cows else -1

c.pump(3.0)
ys = [m[1] for m in moves(c, n, eid)]
check(ys and ys[-1] < G + 1.0, f"it falls to the ground ({ys[-1] if ys else None})")

# a third player joining later sees it as well
late = Client("BotMobs3")
late.join()
late.pump(0.5)
check(eid in mob_adds(late, 0, "minecraft:cow"), "a player who joins later sees it")
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
    c.pump(0.5)
    other.pump(0.1)
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
other.move(X + 3, EYE, Z)
other.pump(0.5)
n2 = len(other.received)
command(c, f"/summon zombie {X + 5} {G} {Z}")
both(other, c, 4.0)
zombies = mob_adds(other, n2, "minecraft:zombie")
check(len(zombies) == 1, "/summon zombie creates a zombie")
check(2 in events(other, n2, victim), "the zombie hits the survival player")
command(other, "/gamemode creative")

n = len(c.received)
command(c, "/summon dragao")
check(any(b"Uso: /summon" in d for p, d in c.received[n:] if p == 0x09), "an unknown mob shows the usage")

# every kind of mob from the behaviour pack can be created
n = len(c.received)
kinds = ["skeleton", "creeper", "spider", "enderman", "wolf", "villager_v2", "bee", "ghast", "blaze", "slime",
         "iron_golem", "cod", "squid", "bat", "horse", "witch", "axolotl", "piglin", "fox", "goat", "warden", "parrot"]
for i, k in enumerate(kinds):
    command(c, f"/summon {k} {X + 40 + (i % 6) * 4} {G} {Z + 40 + (i // 6) * 4}", 0.15)
c.pump(1.0)
other.pump(0.5)
made = [k for k in kinds if mob_adds(c, n, "minecraft:" + k)]
check(len(made) == len(kinds), f"/summon works for the pack's mobs ({len(made)}/{len(kinds)}: missing {set(kinds) - set(made)})")

# a skeleton shoots a survival player
command(other, "/gamemode survival")
other.move(X - 30, EYE, Z)
other.pump(0.5)
n2 = len(other.received)
command(c, f"/summon skeleton {X - 30} {G} {Z + 8}")
both(other, c, 6.0)
arrows = [d for p, d in other.received[n2:] if p == 0x0d and b"minecraft:arrow" in d]
check(len(arrows) >= 1, f"a skeleton shoots arrows ({len(arrows)})")
check(2 in events(other, n2, victim), "the arrows hurt the player")

# a creeper walks up to a survival player and explodes
other.move(X - 60, EYE, Z)
other.pump(0.5)
n2 = len(other.received)
command(c, f"/summon creeper {X - 60} {G} {Z + 3}")
both(other, c, 10.0)
boom = any(p == 0x7b and b"explode" in d for p, d in other.received[n2:])
check(boom, "a creeper explodes next to a survival player")
command(other, "/gamemode creative")

# the ender dragon has no gravity: it flies up into its circling pattern
n = len(c.received)
command(c, f"/summon ender_dragon {X} {G + 2} {Z - 60}")
both(c, other, 5.0)
dragons = mob_adds(c, n, "minecraft:ender_dragon")
check(len(dragons) == 1, "/summon ender_dragon creates it")
ys = [m[1] for m in moves(c, n, dragons[0])] if dragons else []
check(ys and max(ys) > G + 8, f"the dragon flies up ({max(ys) if ys else None})")

# a pig to find again after the restart (run_tests.sh restarts and runs "check")
c.move(X + 10, EYE, Z + 12)  # close by: farther than 32 blocks it may despawn (vanilla)
c.pump(0.3)
command(c, f"/summon pig {X + 10} {G} {Z + 10}")
print("  waiting for the autosave...")
for _ in range(6):
    c.pump(0.8)
    other.pump(0.2)

print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
