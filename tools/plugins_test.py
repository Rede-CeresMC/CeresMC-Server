"""Plugins (python tools/plugins_test.py): the example plugin (plugins-exemplo/MenuExemplo, built to
WebAssembly) registers /menu; the menu is an invisible entity only this player knows about, clicks cannot take the
items and reach the plugin, a teleport takes the menu along, and /reload loads the plugin again."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, svarint, string, read_uvarint

CHEST = block_id("minecraft:chest")
HOTBAR = 28
CURSOR = 59
MENU = 7

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line, wait=0.8):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(wait)

def slot(container, s):
    return bytes([container, 0, s]) + struct.pack("<i", 0)

counter = [-900]
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

def texts(c, since):
    return [d for p, d in c.received[since:] if p == 0x09]

def chest_updates(c, since):
    """positions where the client was told to show a chest"""
    out = []
    for p, d in c.received[since:]:
        if p != 0x15:
            continue
        x = (lambda v: (v >> 1) ^ -(v & 1))(read_uvarint(d, 0)[0])
        y, q = read_uvarint(d, read_uvarint(d, 0)[1])
        z = (lambda v: (v >> 1) ^ -(v & 1))(read_uvarint(d, q)[0])
        rid, _ = read_uvarint(d, read_uvarint(d, q)[1])
        out.append(((x, y, z), rid))
    return out

def opens(c, since):
    return [d for p, d in c.received[since:] if p == 0x2e]

c = Client("BotMenu")
c.join()
c.move(380.5, 71.62, 280.5)
c.pump(0.5)
cmds = [d for p, d in c.received if p == 0x4c]
check(cmds and b"menu" in cmds[-1], "the plugin's /menu is in the command list")

n = len(c.received)
command(c, "/plugins")
check(any(b"MenuExemplo" in t for t in texts(c, n)), "/plugins lists MenuExemplo")

def adds(c, since):
    """(entity id, AddActor payload) of chest minecart entities"""
    out = []
    for p, d in c.received[since:]:
        if p == 0x0d and b"minecraft:chest_minecart" in d:
            v, _ = read_uvarint(d, 0)
            out.append(((v >> 1) ^ -(v & 1), d))
    return out

def opened_entity(d):
    """entity id of a ContainerOpen"""
    p = 2
    for _ in range(3):
        _, p = read_uvarint(d, p)
    v, _ = read_uvarint(d, p)
    return (v >> 1) ^ -(v & 1)

n = len(c.received)
command(c, "/menu", 1.0)
check(not [1 for _, rid in chest_updates(c, n) if rid & 0xffffffff == CHEST & 0xffffffff], "no chest block is placed in the world")
a = adds(c, n)
check(len(a) == 1 and b"Loja do Servidor" in a[0][1], "an invisible entity holds the menu, titled")
eid = a[0][0] if a else None
o = opens(c, n)
check(o and o[-1][1] == 0 and opened_entity(o[-1]) == eid, "the window opens on that entity")
window = o[-1][0] if o else 0
contents = [d for p, d in c.received[n:] if p == 0x31 and read_uvarint(d, 0)[0] == window]
check(bool(contents), "the menu items are sent")

# clicking the diamond: refused (nothing taken), the plugin gives one
n = len(c.received)
taken = request(c, [take(1, slot(MENU, 11), slot(CURSOR, 0))])
check(not taken, "taking an item out of the menu is refused")
c.pump(1.0)
check(any("ganhou 1 diamante".encode() in t for t in texts(c, n)), "the click reached the plugin (on_menu_click)")
check(any(p == 0x32 for p, _ in c.received[n:]), "the plugin gave a diamond (InventorySlot)")

# teleport: the entity goes along and the window stays open
n = len(c.received)
request(c, [take(1, slot(MENU, 13), slot(CURSOR, 0))])
c.pump(1.0)
check(any(p == 0x13 for p, _ in c.received[n:]), "clicking the pearl teleports the player")
check(any(p == 0x12 for p, _ in c.received[n:]), "the menu entity moves with the player")
check(not any(p == 0x2f for p, _ in c.received[n:]) and not opens(c, n), "the window is not closed or reopened")

# walking / flying away: the entity keeps following
c.move(380.5, 81.62, 280.5)  # the client arrives at the teleport destination
c.pump(0.3)
c.move(395.5, 90.62, 300.5)
n = len(c.received)
c.pump(0.5)
check(any(p == 0x12 for p, _ in c.received[n:]), "moving far away, the entity follows")
check(not any(p == 0x2f for p, _ in c.received[n:]), "and the menu stays open")

# the player closes it: the entity goes away and the plugin is told
n = len(c.received)
c.send(uvarint(0x2f) + bytes([window, 0, 0]))
c.pump(1.0)
check(any(p == 0x0e for p, _ in c.received[n:]), "closing removes the entity")
check(any("Menu loja fechado".encode() in t for t in texts(c, n)), "the plugin is told the menu closed")

# pages: the next-page arrow changes the slots without closing the window
n = len(c.received)
command(c, "/menupaginas", 1.0)
o = opens(c, n)
check(bool(o), "the paged menu opens")
window = o[-1][0] if o else 0
n = len(c.received)
request(c, [take(1, slot(MENU, 26), slot(CURSOR, 0))])
c.pump(1.0)
updates = [d for p, d in c.received[n:] if p == 0x32 and read_uvarint(d, 0)[0] == window]
check(len(updates) >= 20, f"the next page replaces the slots ({len(updates)} slots sent)")
check(not opens(c, n) and not any(p == 0x2f for p, _ in c.received[n:]), "the window stays open while changing pages")

# /reload: the plugin comes back and its menu was closed
n = len(c.received)
command(c, "/reload", 0.5)
c.pump(15.0, lambda: any("recarregados".encode() in t for t in texts(c, n)))
check(any("Plugins recarregados: 2 de 2".encode() in t for t in texts(c, n)), "/reload loads the plugin again")
check(any(p == 0x2f for p, _ in c.received[n:]), "the open menu was closed by the reload")
n = len(c.received)
command(c, "/menu", 1.0)
check(bool(opens(c, n)), "/menu works after the reload")

n = len(c.received)
command(c, "/plugin disable MenuExemplo")
command(c, "/menu")
check(any("desconhecido".encode() in t for t in texts(c, n)), "a disabled plugin's command is gone")
command(c, "/plugin enable MenuExemplo")

print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
