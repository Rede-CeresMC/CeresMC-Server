"""Plugins (python tools/plugins_test.py): the example plugin (plugins-exemplo/MenuExemplo, built to
WebAssembly) registers /menu; the menu is a chest only this player sees, clicks cannot take the
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

n = len(c.received)
command(c, "/menu", 1.0)
chests = [pos for pos, rid in chest_updates(c, n) if rid & 0xffffffff == CHEST & 0xffffffff]
check(len(chests) == 1, f"a chest is shown only to the player, under them ({chests})")
check(any(p == 0x38 and b"Loja do Servidor" in d for p, d in c.received[n:]), "the chest gets the menu title")
o = opens(c, n)
check(o and o[-1][1] == 0, "the chest window opens")
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

# teleport: the menu goes along
n = len(c.received)
request(c, [take(1, slot(MENU, 13), slot(CURSOR, 0))])
c.pump(1.0)
moves = [d for p, d in c.received[n:] if p == 0x13]
check(bool(moves), "clicking the pearl teleports the player")
new_chests = [pos for pos, rid in chest_updates(c, n) if rid & 0xffffffff == CHEST & 0xffffffff]
check(new_chests and new_chests[-1][1] >= chests[0][1] + 9 if chests else False, f"the chest moved with the player ({new_chests})")
o2 = opens(c, n)
check(bool(o2), "the menu opened again at the destination")
window = o2[-1][0] if o2 else window
c.move(380.5, 81.62, 280.5)
c.pump(0.3)

# the player closes it: the real block comes back and the plugin is told
n = len(c.received)
c.send(uvarint(0x2f) + bytes([window, 0, 0]))
c.pump(1.0)
restored = [rid for pos, rid in chest_updates(c, n) if new_chests and pos == new_chests[-1]]
check(restored and restored[-1] & 0xffffffff != CHEST & 0xffffffff, "closing puts the real block back")
check(any("Menu loja fechado".encode() in t for t in texts(c, n)), "the plugin is told the menu closed")

# 54 slots
n = len(c.received)
command(c, "/menugrande", 1.5)
chests = [pos for pos, rid in chest_updates(c, n) if rid & 0xffffffff == CHEST & 0xffffffff]
check(len(chests) == 2, f"the big menu is a double chest ({chests})")
o = opens(c, n)
check(bool(o), "the big menu opens")

# /reload: the plugin comes back and its menu was closed
n = len(c.received)
command(c, "/reload", 0.5)
c.pump(15.0, lambda: any("recarregados".encode() in t for t in texts(c, n)))
check(any("Plugins recarregados: 1 de 1".encode() in t for t in texts(c, n)), "/reload loads the plugin again")
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
