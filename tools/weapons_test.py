"""Weapons (python tools/weapons_test.py): a crossbow loads after 1.25 s of holding (the item
gets its chargedItem), fires on the next use and the arrow hurts; a trident is thrown, hurts and
leaves the hand."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, update_blocks, block_id, uvarint, svarint, string, read_uvarint

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


X, Z = 336.5, 262.5
archer = Client("BotArcher")
archer.join()
target = Client("BotTarget")
target.join()
archer.move(X, 71.62, Z)
target.move(X, 71.62, Z + 4)
archer.pump(0.5); target.pump(0.5)
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

def pump_both(seconds):
    for _ in range(int(seconds / 0.1)):
        archer.pump(0.08); target.pump(0.02)

# crossbow: charge, load, fire
command(archer, "/clear")
give(archer, "crossbow 1")
give(archer, "arrow 4")
n = len(archer.received)
click_air(archer)
pump_both(1.5)
release(archer)
pump_both(0.5)
loaded = any(p == 0x32 and b"chargedItem" in d for p, d in archer.received[n:])
check(loaded, "holding a crossbow for 1.25 s loads it (chargedItem)")
n2 = len(target.received)
click_air(archer)
pump_both(1.0)
check(any(p == 0x0d and b"minecraft:arrow" in d for p, d in target.received[n2:]), "the loaded crossbow fires an arrow")
hp = health(target, n2)
check(hp is not None and hp < 20, f"the crossbow arrow hurts (health {hp})")
pump_both(3.0)  # heal and lose the invulnerability frames

# crossbow with Piercing: the arrow goes through a pig and still hurts the player behind
command(archer, "/clear")
give(archer, "crossbow 1")
command(archer, "/enchant @s piercing 1")
give(archer, "arrow 4")
command(archer, f"/summon pig {X} 70 {Z + 2} spawn_adult")
click_air(archer)
pump_both(1.5)
release(archer)
pump_both(0.5)
pump_both(3.0)  # lose the invulnerability frames of the last hit
n2 = len(target.received)
click_air(archer)
pump_both(1.0)
hp = health(target, n2)
check(hp is not None and hp < 20, f"a piercing arrow goes through a mob and hurts the player behind (health {hp})")
pump_both(3.0)

# trident: hold and let go to throw it
command(archer, "/clear")
command(archer, "/effect BotTarget instant_health 1 4")
pump_both(0.5)
give(archer, "trident 1")
target.move(X, 71.62, Z + 4)
n2 = len(target.received)
na = len(archer.received)
click_air(archer)
pump_both(0.8)
release(archer)
pump_both(1.0)
check(any(p == 0x0d and b"thrown_trident" in d for p, d in target.received[n2:]), "a trident is thrown (thrown_trident)")
hp = health(target, n2)
check(hp is not None and hp <= 12.5, f"the trident hurts like vanilla, 8 damage (health {hp})")

# wind charge: thrown, bursts (the target is healed first: the trident left it low)
command(archer, "/effect BotTarget instant_health 1 4")
pump_both(0.5)
command(archer, "/clear")
give(archer, "wind_charge 4")
n2 = len(target.received)
na2 = len(archer.received)
click_air(archer)
pump_both(1.0)
check(any(p == 0x0d and b"wind_charge_projectile" in d for p, d in target.received[n2:]), "a wind charge is thrown")
check(any(p == 0xb0 and b"wind_charge" in d for p, d in archer.received[na2:]), "the thrower's wind charge shows its cooldown")

def level_events(c, since):
    out = []
    for pid, d in c.received[since:]:
        if pid == 0x19:
            v, _ = read_uvarint(d, 0)
            out.append((v >> 1) ^ -(v & 1))
    return out

check(3614 in level_events(target, n2), "the wind charge bursts (wind explosion particles, LevelEvent 3614)")
check(any(p == 0x28 for p, _ in target.received[n2:]), "the wind charge burst throws the target back (SetActorMotion)")

# bottle o' enchanting: thrown, breaks and drops experience orbs
command(archer, "/clear")
give(archer, "experience_bottle 1")
n2 = len(target.received)
click_air(archer)
pump_both(2.5)
check(any(p == 0x0d and b"xp_bottle" in d for p, d in target.received[n2:]), "a bottle o' enchanting is thrown")
check(any(p == 0x0d and b"xp_orb" in d for p, d in target.received[n2:]), "the bottle breaks into experience orbs")

# mace smash in creative: hitting while falling makes the ground dust (LevelEvent 9815)
def attack(c, eid):
    pk = uvarint(0x1e) + svarint(0) + bytes(1) + uvarint(3) + uvarint(0)
    pk += uvarint(eid) + uvarint(1) + svarint(0) + bytes(1) + bytes(8) + struct.pack("<fff", *c.pos) + struct.pack("<fff", 0, 0, 0)
    c.send(pk)

command(archer, "/effect BotTarget instant_health 1 4")
pump_both(0.5)
command(archer, "/clear")
give(archer, "mace 1")
command(archer, "/gamemode creative")
def eid_of(cl):
    for pid, d in cl.received:
        if pid == 0x0b:
            v, _ = read_uvarint(d, 0)
            return (v >> 1) ^ -(v & 1)

target_eid = eid_of(target)
archer.move(X, 71.62 + 5, Z)
pump_both(0.4)
archer.move(X, 71.62 + 1.5, Z)
archer.pump(0.05)
n2 = len(target.received)
if target_eid is not None:
    attack(archer, target_eid)
pump_both(0.8)
check(target_eid is not None and 9815 in level_events(target, n2), "a falling hit with the mace in creative makes the smash dust")
archer.move(X, 71.62, Z)
pump_both(0.5)
command(archer, "/gamemode survival")

# fishing rod: cast a bobber, use again to reel it in
command(archer, "/clear")
give(archer, "fishing_rod 1")
n2 = len(target.received)
click_air(archer)
pump_both(1.0)
hooks = [d for p, d in target.received[n2:] if p == 0x0d and b"fishing_hook" in d]
check(len(hooks) == 1, "a fishing rod casts a bobber")
n3 = len(target.received)
click_air(archer)
pump_both(0.5)
check(any(p == 0x0e for p, _ in target.received[n3:]), "using the rod again reels the bobber in")

# book and quill: write a page, sign it
command(archer, "/clear")
give(archer, "writable_book 1")
archer.send(uvarint(0x61) + svarint(0) + uvarint(0) + svarint(0) + string("Ola mundo") + string(""))
archer.pump(0.3)
n2 = len(archer.received)
archer.send(uvarint(0x61) + svarint(0) + uvarint(4) + string("Diario") + string("x") + string(""))
pump_both(0.8)
signed = [d for p, d in archer.received[n2:] if p == 0x32 and b"Diario" in d]
check(signed and b"Ola mundo" in signed[0], "signing a book and quill makes a written book with its pages")

# powder snow bucket places powder snow
command(archer, "/clear")
give(archer, "powder_snow_bucket 1")
n2 = len(archer.received)
archer.place_block((int(X), 69, int(Z) + 2), 1, 0)
pump_both(0.8)
check(any(p == 0x15 for p, _ in archer.received[n2:]), "a powder snow bucket puts powder snow down")

# item frame: placed on the floor, a diamond goes in
command(archer, "/clear")
give(archer, "frame 1")
archer.place_block((int(X), 69, int(Z) + 1), 1, block_id("minecraft:frame"))
pump_both(0.5)
command(archer, "/clear")
give(archer, "diamond 1")
n2 = len(archer.received)
archer.place_block((int(X), 70, int(Z) + 1), 1, 0)
pump_both(0.8)
check(any(p == 0x38 and b"minecraft:diamond" in d for p, d in archer.received[n2:]), "an item frame takes the held item")

# spear stab (UseItem "use as attack"): misses when walking, hits a pig in front when sprinting
def use_as_attack(c):
    item = struct.pack("<hH", 0, 0) + uvarint(0) + bytes(1) + uvarint(0) + uvarint(0)
    data = svarint(3) + bytes([1]) + svarint(0) * 3 + bytes([255]) + svarint(0) + bytes(1) + item
    data += struct.pack("<fff", *c.pos) + struct.pack("<fff", 0, 0, 0) + uvarint(0) + bytes([1, 0])
    c.send(uvarint(0x1e) + svarint(0) + bytes(1) + uvarint(2) + uvarint(0) + data)

def input_flag(c, flag):
    x, y, z = c.pos
    body = struct.pack("<ff", 0, 0) + struct.pack("<fff", x, y, z) + struct.pack("<fff", 0, 0, 0)
    body += uvarint(1) + svarint(flag) + uvarint(1) + uvarint(0) + svarint(0) + struct.pack("<ff", 0, 0)
    body += uvarint(1) + struct.pack("<fff", 0, 0, 0)
    body += bytes(3)
    body += bytes(2) + struct.pack("<ff", 0, 0) + struct.pack("<fff", 0, 0, 0) + struct.pack("<ff", 0, 0)
    c.send(uvarint(0x90) + body)

command(archer, "/clear")
give(archer, "wooden_spear 1")
command(archer, f"/summon pig {X} 70 {Z + 3} spawn_adult")
pump_both(1.0)
n2 = len(archer.received)
use_as_attack(archer)
pump_both(1.0)
walk_hits = [d for p, d in archer.received[n2:] if p == 0x1b]
input_flag(archer, 25)  # START_SPRINTING
pump_both(0.5)
n2 = len(archer.received)
use_as_attack(archer)
pump_both(1.0)
sprint_hits = [d for p, d in archer.received[n2:] if p == 0x1b]
check(not walk_hits and sprint_hits, f"a spear stab misses walking and hits a mob in front when sprinting ({len(walk_hits)}, {len(sprint_hits)})")
input_flag(archer, 26)  # STOP_SPRINTING
command(archer, "/kill @e[type=pig]")

# brush: four strokes on suspicious sand turn it into sand
SUS = (int(X) + 1, 70, int(Z))
command(archer, "/clear")
give(archer, "stone 1")
archer.place_block((int(X), 69, int(Z)), 5, block_id("minecraft:stone"))  # east of the floor block
pump_both(0.5)
command(archer, "/clear")
give(archer, "suspicious_sand 1")
archer.place_block((SUS[0], 69, SUS[2]), 1, block_id("minecraft:suspicious_sand"))
pump_both(0.5)
command(archer, "/clear")
give(archer, "brush 1")
for _ in range(5):
    archer.place_block(SUS, 1, 0)
    pump_both(0.4)
sand = block_id("minecraft:sand")
check(any(pos == SUS and bid == sand for pos, bid in update_blocks(archer)), "brushing suspicious sand four times turns it into sand")

# fences next to each other connect (the server sends the connection states)
command(archer, "/clear")
give(archer, "oak_fence 2")
F1, F2 = (int(X), 70, int(Z) + 1), (int(X), 70, int(Z) + 2)
archer.place_block((F1[0], 69, F1[2]), 1, block_id("minecraft:oak_fence"))
pump_both(0.4)
archer.place_block((F2[0], 69, F2[2]), 1, block_id("minecraft:oak_fence"))
pump_both(0.6)
plain = block_id("minecraft:oak_fence")
last = {pos: bid for pos, bid in update_blocks(archer)}
check(F1 in last and F2 in last and last[F1] != plain and last[F2] != plain, "two fences side by side connect (connection states)")
archer.break_block(F1)
archer.break_block(F2)

# glow berries under an overhang plant cave vines; breaking the overhang drops the vines;
# a sea pickle placed out of water is dead (not the default, lit state)
command(archer, "/clear")
give(archer, "stone 3")
STONE = block_id("minecraft:stone")
n2 = len(archer.received)
archer.place_block((int(X) + 3, 69, int(Z) + 1), 1, STONE)
pump_both(0.4)
landed = [q for q, b in update_blocks(archer)[-50:] if b == STONE and q[0] == int(X) + 3 and q[2] == int(Z) + 1]
y0 = landed[-1][1] if landed else 70
P1, P2, OV = (int(X) + 3, y0, int(Z) + 1), (int(X) + 3, y0 + 1, int(Z) + 1), (int(X) + 4, y0 + 1, int(Z) + 1)
archer.place_block(P1, 1, STONE)
pump_both(0.3)
archer.place_block(P2, 5, STONE)
pump_both(0.4)
command(archer, "/clear")
give(archer, "glow_berries 1")
VINE = (OV[0], OV[1] - 1, OV[2])
archer.place_block(OV, 0, 0)
pump_both(0.6)
last = {pos: bid for pos, bid in update_blocks(archer)}
check(OV in last and last[OV] == STONE, "an overhanging stone is placed")
check(VINE in last and last[VINE] == block_id("minecraft:cave_vines"), "glow berries plant cave vines under a block")
command(archer, "/gamemode creative")
archer.break_block(OV)
pump_both(0.6)
last = {pos: bid for pos, bid in update_blocks(archer)}
check(last.get(VINE) == block_id("minecraft:air"), "the cave vines fall when the block above goes")
command(archer, "/gamemode survival")
command(archer, "/clear")
give(archer, "sea_pickle 1")
PK = (int(X) + 5, 70, int(Z) + 1)
archer.place_block((PK[0], 69, PK[2]), 1, block_id("minecraft:sea_pickle"))
pump_both(0.5)
last = {pos: bid for pos, bid in update_blocks(archer)}
col = [(q, b) for q, b in update_blocks(archer)[-30:] if q[0] == PK[0] and q[2] == PK[2]]
check(col and col[-1][1] not in (block_id("minecraft:sea_pickle"), block_id("minecraft:air")), "a sea pickle placed out of water is dead")
if col:
    PK = col[-1][0]
for q in (P1, P2, PK):
    archer.break_block(q)
pump_both(0.3)
pump_both(0.4)

# map: an empty map used in the air becomes a filled map and its image is sent (0x43);
# a MapInfoRequest (0x44) for its id gets the image again
command(archer, "/clear")
give(archer, "empty_map 1 2")  # an empty locator map (data 2): the map shows its carriers
n2 = len(archer.received)
click_air(archer)
archer.pump(3.0, lambda: any(p == 0x43 for p, _ in archer.received[n2:]))
maps = [d for p, d in archer.received[n2:] if p == 0x43]
check(maps and any(p == 0x32 and b"map_uuid" in d for p, d in archer.received[n2:]), "an empty map becomes a filled map with its image")
if maps:
    v, _ = read_uvarint(maps[0], 0)
    map_id = (v >> 1) ^ -(v & 1)
    n2 = len(archer.received)
    archer.send(uvarint(0x44) + uvarint(map_id << 1 if map_id >= 0 else (-map_id << 1) - 1) + struct.pack("<I", 0))
    archer.pump(3.0, lambda: any(p == 0x43 for p, _ in archer.received[n2:]))
    check(any(p == 0x43 for p, _ in archer.received[n2:]), "MapInfoRequest gets the map image")
    archer.pump(1.0)
    small = [d for p, d in archer.received[n2:] if p == 0x43 and len(d) < 1000]
    check(small, "the locator map also gets the player markers")
    painted = sum(1 for i in range(len(maps[0]) - 4 * 16384, len(maps[0]), 4) if maps[0][i + 3] != 0)
    check(painted > 1000, f"the map shows the ground ({painted} pixels drawn)")

archer.close(); target.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
