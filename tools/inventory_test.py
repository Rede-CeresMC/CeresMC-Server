"""Creative inventory test: take a stack from the creative menu into the hotbar, then split it."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, uvarint, svarint, read_uvarint

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def slot(container, s, stack_id=0):
    return bytes([container]) + b"\x00" + bytes([s]) + struct.pack("<i", stack_id)

def request(req_id, actions):
    body = svarint(req_id) + uvarint(len(actions))
    for ty, data in actions:
        body += uvarint(ty) + b"\x00" + data
    body += uvarint(0) + struct.pack("<i", 0)
    return uvarint(0x93) + uvarint(1) + body

def responses(c):
    out = []
    for pid, d in c.received:
        if pid != 0x94:
            continue
        n, p = read_uvarint(d, 0)
        result = d[p]; p += 1
        rid, p = read_uvarint(d, p); rid = (rid >> 1) ^ -(rid & 1)
        slots = []
        if result == 0 and d[p]:
            p += 1
            nc, p = read_uvarint(d, p)
            for _ in range(nc):
                cid = d[p]; p += 2
                ns, p = read_uvarint(d, p)
                for _ in range(ns):
                    s, count = d[p], d[p + 2]; p += 4
                    sid, p = read_uvarint(d, p)
                    ln, p = read_uvarint(d, p); p += ln + 1
                    ln, p = read_uvarint(d, p); p += ln
                    _, p = read_uvarint(d, p)
                    slots.append((cid, s, count))
        out.append((rid, result, slots))
    return out

c = Client("BotInv")
c.join()
# creative create item #1, take 64 into hotbar slot 0
c.send(request(-2, [(12, uvarint(1) + b"\x01"), (0, b"\x40" + slot(60, 50) + slot(28, 0))]))
c.pump(4.0, lambda: len(responses(c)) >= 1)
r = responses(c)
check(r and r[0][1] == 0 and (28, 0, 64) in r[0][2], f"creative take answered with hotbar slot 0 = 64 ({r[:1]})")
# move 16 of them to hotbar slot 1
c.send(request(-4, [(1, b"\x10" + slot(28, 0) + slot(28, 1))]))
c.pump(4.0, lambda: len(responses(c)) >= 2)
r = responses(c)
check(len(r) > 1 and r[1][1] == 0 and set(r[1][2]) == {(28, 0, 48), (28, 1, 16)}, f"split 64 -> 48 + 16 ({r[1:2]})")
# taking more than there is must be refused
c.send(request(-6, [(0, b"\x40" + slot(28, 1) + slot(28, 2))]))
c.pump(4.0, lambda: len(responses(c)) >= 3)
r = responses(c)
check(len(r) > 2 and r[2][1] != 0, "taking 64 from a slot with 16 is refused")
# what the real client sends: creative create + deprecated "crafting results" + take
results = uvarint(1) + uvarint(1) + b"" + uvarint(len("minecraft:stone")) + b"minecraft:stone" + svarint(0) + struct.pack("<h", 64) + uvarint(0) + uvarint(0) + b""
c.send(request(-8, [(12, uvarint(1) + b""), (18, results), (0, b"@" + slot(60, 50) + slot(28, 5))]))
c.pump(4.0, lambda: len(responses(c)) >= 4)
r = responses(c)
check(len(r) > 3 and r[3][1] == 0 and (28, 5, 64) in r[3][2], f"real client creative take (with crafting results) works ({r[3:4]})")
c.pump(0.5)
check(not any(p == 0x05 for p, _ in c.received), "not disconnected")
c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
