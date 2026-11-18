"""Plugin API (python tools/plugin_api_test.py), with tools/plugin-teste: forms, scoreboard,
bossbar, actionbar, repeating tasks and the cancellable events (chat, block break/place)."""
import sys, os, struct, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, string, read_uvarint, update_blocks

GOLD = block_id("minecraft:gold_block")
DIAMOND = block_id("minecraft:diamond_block")
CLICKED = (390, 69, 290)
TARGET = (390, 69, 290)  # y=69 is air in the flat test world: the click replaces it

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line, wait=0.8):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(wait)

def texts(c, since):
    return [d for p, d in c.received[since:] if p == 0x09]

def said(c, since, text):
    return any(text.encode() in t for t in texts(c, since))

def forms(c, since):
    out = []
    for p, d in c.received[since:]:
        if p == 0x64:
            fid, q = read_uvarint(d, 0)
            n, q = read_uvarint(d, q)
            out.append((fid, json.loads(d[q:q + n])))
    return out

def answer(c, fid, data):
    body = uvarint(fid) + (b"\x01" + string(data) if data is not None else b"\x00") + b"\x00"
    c.send(uvarint(0x65) + body)
    c.pump(0.8)

def block_at(c, since, pos):
    ups = [bid & 0xffffffff for p, bid in update_blocks_since(c, since) if tuple(p) == tuple(pos)]
    return ups[-1] if ups else None

def update_blocks_since(c, since):
    saved = c.received
    c.received = saved[since:]
    try:
        return update_blocks(c)
    finally:
        c.received = saved

c = Client("BotApi")
c.join()
c.move(390.5, 71.62, 287.5)
c.pump(0.5)

# chat: the plugin cancels messages with "proibido"
n = len(c.received)
c.send_text("mensagem normal")
c.pump(0.6)
check(said(c, n, "mensagem normal"), "a normal chat message goes through")
n = len(c.received)
c.send_text("isto e proibido")
c.pump(0.6)
check(not said(c, n, "proibido"), "a message the plugin cancels is not sent")

# simple form: a button, a closed form and a forged answer
n = len(c.received)
command(c, "/formteste")
f = forms(c, n)
check(f and f[-1][1].get("type") == "form" and len(f[-1][1]["buttons"]) == 2, "the simple form arrives")
if f:
    n = len(c.received)
    answer(c, f[-1][0], "1")
    check(said(c, n, "form botoes: botao 1"), "the button answer reaches the plugin")
    n = len(c.received)
    answer(c, f[-1][0], "0")
    check(not said(c, n, "form botoes"), "a form can only be answered once")
n = len(c.received)
command(c, "/formteste")
f = forms(c, n)
if f:
    n = len(c.received)
    answer(c, f[-1][0], "7")
    check(said(c, n, "form botoes: fechado"), "a button that does not exist counts as closed")

# custom form
n = len(c.received)
command(c, "/formcampos")
f = forms(c, n)
check(f and f[-1][1].get("type") == "custom_form" and len(f[-1][1]["content"]) == 3, "the custom form arrives")
if f:
    n = len(c.received)
    answer(c, f[-1][0], '["Ana", true, 7]')
    check(said(c, n, 'form campos: campos ["Ana",true,7]'), "the field values reach the plugin")

# scoreboard, bossbar, actionbar
n = len(c.received)
command(c, "/placar")
ids = [p for p, _ in c.received[n:]]
check(0x6b in ids and 0x6c in ids, "the sidebar scoreboard is sent")
scores = [d for p, d in c.received[n:] if p == 0x6c]
check(scores and scores[-1].count(b"Linha um") == 2, "two equal lines stay two lines")
check(0x4a in ids, "the bossbar is sent")
check(any(p == 0x58 and b"Barra" in d for p, d in c.received[n:]), "the actionbar is sent")

# repeating task, cancelled by the plugin after two runs
n = len(c.received)
command(c, "/tarefa", 0.3)
c.pump(2.0)
check(said(c, n, "tarefa 1") and said(c, n, "tarefa 2"), "the repeating task runs")
check(not said(c, n, "tarefa 3"), "and stops when the plugin cancels it")

# blocks: gold cannot be broken, diamond cannot be placed
command(c, "/gamemode creative")
c.break_block(TARGET)
c.pump(0.3)
c.place_block(CLICKED, 1, GOLD)
c.pump(0.6)
n = len(c.received)
c.break_block(TARGET)
c.pump(0.8)
check(block_at(c, n, TARGET) == GOLD & 0xffffffff, "breaking the gold block is cancelled (it comes back)")
command(c, "/plugin disable Teste")
c.break_block(TARGET)
c.pump(0.5)
command(c, "/plugin enable Teste")
n = len(c.received)
c.place_block(CLICKED, 1, DIAMOND)
c.pump(0.8)
placed = block_at(c, n, TARGET)
check(placed is not None and placed != DIAMOND & 0xffffffff, "placing a diamond block is cancelled")

print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
