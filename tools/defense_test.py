"""Totem and shield (python tools/defense_test.py)."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, svarint, string, read_uvarint

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

def lowest_health(c, since):
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

def eid_of(c):
    for p, d in c.received:
        if p == 0x0b:
            v, _ = read_uvarint(d, 0)
            return (v >> 1) ^ -(v & 1)

def attack(att, target_eid):
    pk = uvarint(0x1e) + svarint(0) + bytes(1) + uvarint(3) + uvarint(0)
    pk += uvarint(target_eid) + uvarint(1) + svarint(0) + bytes(1) + bytes(8) + struct.pack("<fff", *att.pos) + struct.pack("<fff", 0, 0, 0)
    att.send(pk)

X, Z = 338.5, 262.5
guard = Client("BotGuard")
guard.join()
foe = Client("BotFoe")
foe.join()
guard_eid = eid_of(guard)
command(guard, "/gamemode survival")
command(foe, "/gamemode survival")

# totem: a hit that would kill leaves 1 health instead
command(guard, "/clear")
give(guard, "totem_of_undying 1")
n = len(guard.received)
command(guard, "/effect @s instant_damage 1 10")
guard.pump(1.0)
def actor_events(c, since):
    out = []
    for pid, d in c.received[since:]:
        if pid == 0x1b:
            _, p = read_uvarint(d, 0)
            out.append(d[p])
    return out
check(65 in actor_events(guard, n), f"the totem animation plays (ActorEvent 65) ({actor_events(guard, n)})")
hp = lowest_health(guard, n)
check(hp is not None and hp >= 1.0, f"the totem keeps the player alive ({hp})")
check(not any(p == 0x2d for p, _ in guard.received[n:]), "no respawn: the player did not die")

# shield: sneaking with a shield, facing the attacker, blocks the hit
command(guard, "/effect @s clear")
command(guard, "/clear")
give(guard, "shield 1")
guard.move(X, 71.62, Z + 2, yaw=180.0)   # looks north, towards the attacker
foe.move(X, 71.62, Z)
guard.pump(0.3); foe.pump(0.3)
guard.sneak(True)
guard.pump(0.3)
guard.move(X, 71.62, Z + 2, yaw=180.0)   # the sneak input carried no rotation
guard.pump(1.2)  # the shield goes up on the next server tick
n = len(guard.received)
attack(foe, guard_eid)
guard.pump(1.0); foe.pump(0.2)
hp = lowest_health(guard, n)
check(not any(p == 0x28 for p, _ in guard.received[n:]), "a raised shield blocks the hit (no knockback)")
guard.sneak(False)
guard.pump(0.3)
guard.move(X, 71.62, Z + 2, yaw=180.0)
guard.pump(0.8)
n = len(guard.received)
attack(foe, guard_eid)
guard.pump(1.0); foe.pump(0.2)
check(any(p == 0x28 for p, _ in guard.received[n:]), "without the shield raised the hit lands")

guard.close(); foe.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
