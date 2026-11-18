"""Commands test. The test server's ops.txt lists BotOp; BotUser is a normal player."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, uvarint, svarint, string, read_uvarint

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    origin = string("player") + bytes(16) + string("") + struct.pack("<q", 0)
    c.send(uvarint(0x4d) + string(line) + origin + b"\x00" + string(""))

def texts(c):
    return [d for p, d in c.received if p == 0x09]

def wait_text(c, needle):
    c.pump(1.5, lambda: any(needle.encode() in d for d in texts(c)))
    return any(needle.encode() in d for d in texts(c))

op, user = Client("BotOp"), Client("BotUser")
op.join(); user.join()
check(any(p == 0x4c for p, _ in op.received), "operator receives the command list")

command(op, "/gamemode survival")
op.pump(1.5, lambda: any(p == 0x3e for p, _ in op.received))
gm = [d for p, d in op.received if p == 0x3e]
check(gm and gm[-1][0] == 0, "/gamemode survival sends SetPlayerGameType(0)")

n = len([1 for p, _ in op.received if p == 0x13])
command(op, "/tp 10 80 10")
op.pump(1.5, lambda: len([1 for p, _ in op.received if p == 0x13]) > n)
moves = [d for p, d in op.received if p == 0x13]
if moves:
    _, q = read_uvarint(moves[-1], 0)
    x, y, z = struct.unpack("<fff", moves[-1][q:q + 12])
check(moves and abs(x - 10) < 0.01 and abs(y - 81.62) < 0.01, "/tp 10 80 10 moves the operator")
check(any(p == 0x79 for p, _ in op.received), "teleport sends a chunk publisher update")

command(op, "/list")
check(wait_text(op, "BotUser"), "/list shows the other player")

command(op, "/time set night")
op.pump(1.0, lambda: any(p == 0x0a for p, _ in op.received))
check(any(p == 0x0a for p, _ in op.received), "/time set night sends SetTime")

command(user, "/gamemode creative")
check(wait_text(user, "permiss"), "a non-operator is refused")

command(op, "/naoexiste")
check(wait_text(op, "desconhecido"), "unknown commands get an answer")

op.close(); user.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
