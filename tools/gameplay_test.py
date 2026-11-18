"""Gameplay systems test: real items, equipment, animations/sneaking, item drops & pickup,
block orientation & interaction, PvP combat, weather/titles/worlds and moderation."""
import sys, os, struct, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, uvarint, svarint, string, read_uvarint

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    origin = string("player") + bytes(16) + string("") + struct.pack("<q", 0)
    c.send(uvarint(0x4d) + string(line) + origin + b"\x00" + string(""))

def wait_pkt(c, pid, timeout=4.0):
    n = sum(1 for p, _ in c.received if p == pid)
    c.pump(timeout, lambda: sum(1 for p, _ in c.received if p == pid) > n)
    return [d for p, d in c.received[0:] if p == pid]

def auth_input_with_flags(c, flags):
    x, y, z = c.pos
    body = struct.pack("<ff", 0, 0) + struct.pack("<fff", x, y, z) + struct.pack("<fff", 0, 0, 0)
    body += uvarint(len(flags)) + b"".join(svarint(f) for f in flags)
    body += uvarint(1) + uvarint(0) + svarint(0) + struct.pack("<ff", 0, 0)
    body += uvarint(1) + struct.pack("<fff", 0, 0, 0)
    body += b"\x00\x00\x00\x00\x00" + struct.pack("<ff", 0, 0) + struct.pack("<fff", 0, 0, 0) + struct.pack("<ff", 0, 0)
    return uvarint(0x90) + body

SPAWN = (256.5, 71.62, 256.5)
op = Client("BotOp")
peer = Client("BotPeer")
op.join()
peer.join()
peer_eid = None
for p, d in peer.received:
    if p == 0x0b:
        v, _ = read_uvarint(d, 0)
        peer_eid = (v >> 1) ^ -(v & 1)
assert peer_eid is not None, "could not read BotPeer entity_id from StartGame"
op.move(*SPAWN)
peer.move(258.5, 71.62, 256.5)
op.pump(0.5); peer.pump(0.5)

# 1. /give real item -> InventorySlot (0x32) to BotOp and MobEquipment (0x1f) to BotPeer
command(op, "/clear")  # inventories are saved per player: start from an empty one
op.pump(0.5)
peer.received.clear(); op.received.clear()
command(op, "/give @s diamond_sword 1")
op.pump(3.0, lambda: any(p == 0x32 for p, _ in op.received))
peer.pump(3.0, lambda: any(p == 0x1f for p, _ in peer.received))
check(any(p == 0x32 for p, _ in op.received), "/give sends InventorySlot with real item")
check(any(p == 0x1f for p, _ in peer.received), "other player sees held item via MobEquipment")

# 2. Hotbar switch (MobEquipment 0x1f from client)
peer.received.clear()
empty_item = bytes(8)
op.send(uvarint(0x1f) + uvarint(1) + empty_item + bytes([1, 1, 0]))
peer.pump(3.0, lambda: any(p == 0x1f for p, _ in peer.received))
check(any(p == 0x1f for p, _ in peer.received), "switching hotbar slot broadcasts MobEquipment")
# switch back to slot 0 (diamond sword)
op.send(uvarint(0x1f) + uvarint(1) + empty_item + bytes([0, 0, 0]))
op.pump(0.3); peer.pump(0.3)

# 3. Sneaking state (START_SNEAKING = 27) -> SetActorData (0x27)
peer.received.clear()
op.send(auth_input_with_flags(op, [27]))
peer.pump(3.0, lambda: any(p == 0x27 for p, _ in peer.received))
check(any(p == 0x27 for p, _ in peer.received), "sneaking broadcasts SetActorData")
op.send(auth_input_with_flags(op, [28]))  # STOP_SNEAKING
op.pump(0.3); peer.pump(0.3)

# 4. Arm swing (AnimatePacket 0x2c)
peer.received.clear()
op.send(uvarint(0x2c) + bytes([1]) + uvarint(1) + struct.pack("<f", 0.0) + b"\x00\x00")
peer.pump(3.0, lambda: any(p == 0x2c for p, _ in peer.received))
check(any(p == 0x2c for p, _ in peer.received), "Animate arm swing broadcasts to other players")

# 5. Block orientation + interactive trapdoor toggle
OAK_LOG = block_id("minecraft:oak_log")
TRAPDOOR = block_id("minecraft:trapdoor")
BASE = (262, 69, 262)
op.move(260.5, 71.62, 262.5)
peer.move(260.5, 71.62, 260.5)
op.pump(0.3); peer.pump(0.3)
op.break_block((262, 70, 262))
op.break_block((263, 70, 262))
op.place_block(BASE, 1, block_id("minecraft:stone"))  # BASE is air: this puts a solid support there
op.pump(0.5); peer.pump(0.5)
peer.received.clear()
# Place oak log on east face (face=5) of (262, 70, 262) after placing anchor
op.place_block(BASE, 1, OAK_LOG)
peer.pump(2.0, lambda: len(update_blocks(peer)) >= 1)
peer.received.clear()
op.place_block((262, 70, 262), 5, OAK_LOG)  # east face -> pillar_axis = x (different runtime id than y!)
peer.pump(3.0, lambda: any(pos == (263, 70, 262) for pos, _ in update_blocks(peer)))
placed_logs = [bid for pos, bid in update_blocks(peer) if pos == (263, 70, 262)]
check(placed_logs and placed_logs[-1] != OAK_LOG, "placing oak_log on side face orients pillar_axis")

# Place trapdoor and right-click it to toggle open_bit
op.break_block((262, 70, 262)); op.break_block((263, 70, 262))
op.pump(0.4); peer.pump(0.4)
peer.received.clear()
op.place_block(BASE, 1, TRAPDOOR)
peer.pump(3.0, lambda: any(pos == (262, 70, 262) for pos, _ in update_blocks(peer)))
before_toggle = [bid for pos, bid in update_blocks(peer) if pos == (262, 70, 262)][-1]
peer.received.clear()
op.place_block((262, 70, 262), 1, TRAPDOOR)  # clicking interactive trapdoor toggles it!
peer.pump(3.0, lambda: any(pos == (262, 70, 262) for pos, _ in update_blocks(peer)))
after_toggle = [bid for pos, bid in update_blocks(peer) if pos == (262, 70, 262)]
check(after_toggle and after_toggle[-1] != before_toggle, "right-clicking trapdoor toggles open_bit")
op.break_block((262, 70, 262))

# 6. Survival break drops ItemEntity (0x0f) + automatic pickup (0x11)
command(op, "/gamemode survival @a")
op.pump(0.5); peer.pump(0.5)
command(op, "/clear")  # empty hand slot 0 so the dirt lands there (the bot places from slot 0)
op.pump(0.5)
command(op, "/give BotOp dirt 2")
op.pump(1.0)
op.received.clear(); peer.received.clear()
op.place_block(BASE, 1, block_id("minecraft:dirt"))
op.pump(1.0); peer.pump(1.0)
# survival mining takes time: dirt by hand needs 0.75 s
from bedrock_client import _auth_input
op.send(_auth_input(op, [(0, (262, 70, 262), 1)]))
op.pump(0.9)
op.send(_auth_input(op, [(26, (262, 70, 262), 1)]))
op.pump(3.0, lambda: any(p == 0x0f for p, _ in op.received))
check(any(p == 0x0f for p, _ in op.received), "breaking block in survival spawns AddItemActor (0x0f)")
# Move onto dropped item to pick it up after 10-tick pickup delay
op.move(262.5, 71.62, 262.5)
op.pump(3.0, lambda: any(p == 0x11 for p, _ in op.received))
check(any(p == 0x11 for p, _ in op.received), "walking over dropped item triggers TakeItemActor (0x11)")

# 7. PvP attack (UseItemOnEntity type=3, ACTION_ATTACK=1) -> ActorEvent HURT (0x1b) + Knockback (0x28)
peer.move(263.5, 71.62, 262.5)
op.pump(0.3); peer.pump(0.3)
peer.received.clear()
atk_item = bytes(8)
atk = uvarint(0x1e) + svarint(0) + b"\x00" + uvarint(3) + uvarint(0)
atk += uvarint(peer_eid) + uvarint(1) + svarint(0) + b"\x00" + atk_item + struct.pack("<fff", *op.pos) + struct.pack("<fff", 0, 0, 0)
op.send(atk)
peer.pump(3.0, lambda: any(p == 0x1b for p, _ in peer.received) and any(p == 0x28 for p, _ in peer.received))
check(any(p == 0x1b for p, _ in peer.received), "PvP attack triggers hurt ActorEvent (0x1b)")
check(any(p == 0x28 for p, _ in peer.received), "PvP attack applies knockback SetActorMotion (0x28)")

# 8. Weather, Difficulty, Title, Toast, Multiworld creation
peer.received.clear()
command(op, "/weather rain 60")
peer.pump(3.0, lambda: any(p == 0x19 for p, _ in peer.received))
check(any(p == 0x19 for p, _ in peer.received), "/weather rain broadcasts LevelEvent START_RAIN")
command(op, "/weather clear")

peer.received.clear()
command(op, "/difficulty hard")
peer.pump(3.0, lambda: any(p == 0x3c for p, _ in peer.received))
check(any(p == 0x3c for p, _ in peer.received), "/difficulty hard broadcasts SetDifficulty")

peer.received.clear()
command(op, "/title BotPeer title Bem-vindo!")
command(op, "/toast BotPeer CeresMC Novo sistema!")
peer.pump(3.0, lambda: any(p == 0x58 for p, _ in peer.received) and any(p == 0xba for p, _ in peer.received))
check(any(p == 0x58 for p, _ in peer.received) and any(p == 0xba for p, _ in peer.received), "/title and /toast deliver UI packets")

command(op, "/gamemode creative @a")
command(op, "/world create arena void")
op.pump(2.0)
op.received.clear()
command(op, "/world arena")
op.pump(4.0, lambda: op.chunks > 0 and any(p == 0x2b for p, _ in op.received))
check(any(p == 0x2b for p, _ in op.received), "/world create + /world arena switches to newly created void world")
command(op, "/world world")
op.pump(2.0)

# 9. Moderation: /ban kicks player and blocks rejoin until /pardon
peer.received.clear()
command(op, "/ban BotPeer Teste de ban")
peer.pump(3.0, lambda: any(p == 0x05 for p, _ in peer.received))
check(any(p == 0x05 for p, _ in peer.received), "/ban disconnects the banned player")
peer.close()

banned = Client("BotPeer")
banned.connect()
banned.send(uvarint(0xc1) + struct.pack(">I", 2225))
banned.pump(3.0, lambda: any(p == 0x05 for p, _ in banned.received))
check(any(p == 0x05 for p, _ in banned.received), "banned player is refused on reconnect")
banned.close()

command(op, "/pardon BotPeer")
op.pump(0.5)
unbanned = Client("BotPeer")
unbanned.join()
check(unbanned.spawned, "/pardon allows the player to join again")

unbanned.close()
op.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
