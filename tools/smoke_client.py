"""Minimal Bedrock client to smoke-test Ceres-RS: RakNet handshake, login and chunk streaming."""
import socket, struct, time, zlib, json, base64, random, sys

HOST, PORT = "127.0.0.1", 19132
MAGIC = bytes.fromhex("00ffff00fefefefefdfdfdfd12345678")
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(2)
addr = (HOST, PORT)

def uvarint(v):
    out = b""
    while True:
        if v < 0x80:
            return out + bytes([v])
        out += bytes([(v & 0x7f) | 0x80]); v >>= 7

def read_uvarint(b, p):
    v = 0; sh = 0
    while True:
        c = b[p]; p += 1
        v |= (c & 0x7f) << sh; sh += 7
        if not c & 0x80: return v, p

def zz(v): return uvarint((v << 1) ^ (v >> 31))

# ---- ping
s.sendto(b"\x01" + struct.pack(">Q", 123) + MAGIC + struct.pack(">Q", 1), addr)
d, _ = s.recvfrom(2048)
n = struct.unpack(">H", d[33:35])[0]
print("PONG:", d[35:35 + n].decode())

# ---- open connection
s.sendto(b"\x05" + MAGIC + b"\x0b" + b"\x00" * (1400 - 18), addr)
d, _ = s.recvfrom(2048); assert d[0] == 0x06, d
mtu = struct.unpack(">H", d[26:28])[0]
s.sendto(b"\x07" + MAGIC + b"\x04" + bytes([~127 & 255, 255, 255, ~1 & 255]) + struct.pack(">H", PORT) + struct.pack(">H", mtu) + struct.pack(">Q", 42), addr)
d, _ = s.recvfrom(2048); assert d[0] == 0x08, d
print("RakNet open ok, mtu", mtu)

seq = 0; msg = 0; order = 0
def send_frame(body):
    global seq, msg, order
    fr = bytes([3 << 5]) + struct.pack(">H", len(body) << 3) + struct.pack("<I", msg)[:3] + struct.pack("<I", order)[:3] + b"\x00" + body
    msg += 1; order += 1
    s.sendto(b"\x84" + struct.pack("<I", seq)[:3] + fr, addr); seq += 1

splits = {}
def recv_bodies(timeout=2.0):
    out = []
    s.settimeout(timeout)
    try:
        d, _ = s.recvfrom(4096)
    except socket.timeout:
        return out
    if d[0] & 0x40 or d[0] & 0x20: return out
    if not d[0] & 0x80: return out
    sq = d[1] | d[2] << 8 | d[3] << 16
    s.sendto(b"\xc0" + struct.pack(">H", 1) + b"\x01" + struct.pack("<I", sq)[:3], addr)
    p = 4
    while p < len(d):
        flags = d[p]; p += 1
        rel = flags >> 5; split = flags & 0x10
        ln = (struct.unpack(">H", d[p:p+2])[0] + 7) // 8; p += 2
        if rel in (2, 3, 4, 6, 7): p += 3
        if rel in (1, 4): p += 3
        if rel in (1, 3, 4, 7): p += 4
        sp = None
        if split:
            cnt, sid, idx = struct.unpack(">IHI", d[p:p+10]); p += 10; sp = (cnt, sid, idx)
        body = d[p:p+ln]; p += ln
        if sp:
            cnt, sid, idx = sp
            parts = splits.setdefault(sid, {})
            parts[idx] = body
            if len(parts) == cnt:
                out.append(b"".join(parts[i] for i in range(cnt))); del splits[sid]
        else:
            out.append(body)
    return out

send_frame(b"\x09" + struct.pack(">QQ", 42, 0) + b"\x00")
for _ in range(5):
    bodies = recv_bodies()
    if any(b[0] == 0x10 for b in bodies): break
else:
    sys.exit("no ConnectionRequestAccepted")
print("ConnectionRequestAccepted ok")
send_frame(b"\x13" + b"\x04\x80\xff\xff\xfe" + struct.pack(">H", PORT) + b"\x00" * 7 * 20 + struct.pack(">QQ", 0, 0))

compression = False
def send_game(packets):
    raw = b"".join(uvarint(len(p)) + p for p in packets)
    if compression:
        c = zlib.compressobj(6, zlib.DEFLATED, -15); raw = b"\x00" + c.compress(raw) + c.flush()
    send_frame(b"\xfe" + raw)

def decode(body):
    raw = body[1:]
    if compression:
        raw = zlib.decompress(raw[1:], -15) if raw[0] == 0 else raw[1:]
    pks = []; p = 0
    while p < len(raw):
        n, p = read_uvarint(raw, p); pks.append(raw[p:p+n]); p += n
    return pks

def jwt(payload):
    enc = lambda o: base64.urlsafe_b64encode(json.dumps(o).encode()).rstrip(b"=").decode()
    return enc({"alg": "ES384"}) + "." + enc(payload) + ".sig"

send_game([uvarint(0xc1) + struct.pack(">I", 2225)])
names = {0x8f: "NetworkSettings", 0x02: "PlayStatus", 0x06: "ResourcePacksInfo", 0x07: "ResourcePackStack", 0x0b: "StartGame",
         0xa2: "ItemRegistry", 0x77: "ActorIdentifiers", 0x7a: "BiomeDefinitionList", 0x139: "JigsawStructureData", 0x151: "VoxelShapes",
         0x46: "ChunkRadiusUpdated", 0x79: "NetworkChunkPublisherUpdate", 0x3a: "LevelChunk", 0x05: "Disconnect"}
chunks = 0; spawned = False; sizes = {}
deadline = time.time() + 20
while time.time() < deadline and not spawned:
    for body in recv_bodies():
        if body[0] != 0xfe: continue
        for pk in decode(body):
            pid, p = read_uvarint(pk, 0); pid &= 0x3ff
            sizes[pid] = len(pk)
            if pid == 0x3a:
                chunks += 1; continue
            print("<-", names.get(pid, hex(pid)), len(pk), "bytes")
            if pid == 0x8f:
                compression = True
                auth = json.dumps({"AuthenticationType": 2, "Certificate": "{}", "Token": ""}).encode()
                cd = jwt({"ThirdPartyName": "TestBot"}).encode()
                blob = struct.pack("<I", len(auth)) + auth + struct.pack("<I", len(cd)) + cd
                send_game([uvarint(0x01) + struct.pack(">I", 2225) + uvarint(len(blob)) + blob])
            elif pid == 0x06:
                send_game([uvarint(0x08) + uvarint(2) + uvarint(0)])
            elif pid == 0x07:
                send_game([uvarint(0x08) + uvarint(3) + uvarint(0)])
            elif pid == 0x0b:
                send_game([uvarint(0x45) + zz(6) + b"\x06"])
            elif pid == 0x02 and pk[p:p+4] == b"\x00\x00\x00\x03":
                print("PLAYER_SPAWN received after", chunks, "chunks")
                send_game([uvarint(0x71) + uvarint(1)])
                spawned = True
            elif pid == 0x05:
                sys.exit("disconnected")
print("chunks:", chunks, "spawned:", spawned)
s.sendto(b"\x84" + struct.pack("<I", seq)[:3] + bytes([0]) + struct.pack(">H", 8) + b"\x15", addr)
