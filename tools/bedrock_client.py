"""Minimal Bedrock client over RakNet, for testing CeresMC Server without the game.

    from bedrock_client import Client
    c = Client("Bot1"); c.join()          # login -> chunks -> spawned
    c.send_text("hello"); c.move(1, 0, 0)
    c.pump(2.0)                           # receive for 2 seconds
    c.received                            # list of (packet_id, payload)
"""
import base64, json, random, socket, struct, time, zlib

MAGIC = bytes.fromhex("00ffff00fefefefefdfdfdfd12345678")
PROTOCOL = 2225

NAMES = {0x02: "PlayStatus", 0x05: "Disconnect", 0x06: "ResourcePacksInfo", 0x07: "ResourcePackStack",
         0x09: "Text", 0x0b: "StartGame", 0x0c: "AddPlayer", 0x0e: "RemoveActor", 0x13: "MovePlayer",
         0x3a: "LevelChunk", 0x3f: "PlayerList", 0x46: "ChunkRadiusUpdated", 0x77: "ActorIdentifiers",
         0x79: "ChunkPublisherUpdate", 0x7a: "BiomeDefinitionList", 0x8f: "NetworkSettings",
         0xa2: "ItemRegistry", 0xbb: "UpdateAbilities", 0x139: "JigsawStructureData", 0x151: "VoxelShapes"}


def uvarint(v):
    out = b""
    while True:
        if v < 0x80:
            return out + bytes([v])
        out += bytes([(v & 0x7f) | 0x80])
        v >>= 7


def read_uvarint(b, p):
    v = sh = 0
    while True:
        c = b[p]; p += 1
        v |= (c & 0x7f) << sh; sh += 7
        if not c & 0x80:
            return v, p


def zz(v):
    return uvarint((v << 1) ^ (v >> 31))


def string(s):
    b = s.encode()
    return uvarint(len(b)) + b


def jwt(payload):
    enc = lambda o: base64.urlsafe_b64encode(json.dumps(o).encode()).rstrip(b"=").decode()
    return enc({"alg": "ES384"}) + "." + enc(payload) + ".sig"


class Client:
    def __init__(self, name, host="127.0.0.1", port=None):
        import os
        port = port or int(os.environ.get("CERES_PORT", "19132"))
        self.name, self.addr = name, (host, port)
        self.s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.seq = self.msg = self.order = 0
        self.splits = {}
        self.compression = False
        self.received = []  # (packet id, payload)
        self.chunks = 0
        self.spawned = False
        self.pos = [0.0, 0.0, 0.0]

    # ---------------------------------------------------------------- raknet
    def ping(self):
        self.s.settimeout(2)
        self.s.sendto(b"\x01" + struct.pack(">Q", 123) + MAGIC + struct.pack(">Q", 1), self.addr)
        d, _ = self.s.recvfrom(2048)
        n = struct.unpack(">H", d[33:35])[0]
        return d[35:35 + n].decode()

    def _frame(self, body):
        fr = bytes([3 << 5]) + struct.pack(">H", len(body) << 3) + struct.pack("<I", self.msg)[:3] \
            + struct.pack("<I", self.order)[:3] + b"\x00" + body
        self.msg += 1; self.order += 1
        self.s.sendto(b"\x84" + struct.pack("<I", self.seq)[:3] + fr, self.addr)
        self.seq += 1

    def _recv(self, timeout):
        out = []
        self.s.settimeout(timeout)
        try:
            d, _ = self.s.recvfrom(4096)
        except socket.timeout:
            return out
        if not d[0] & 0x80 or d[0] & 0x60:
            return out
        sq = d[1] | d[2] << 8 | d[3] << 16
        self.s.sendto(b"\xc0" + struct.pack(">H", 1) + b"\x01" + struct.pack("<I", sq)[:3], self.addr)
        p = 4
        while p < len(d):
            flags = d[p]; p += 1
            rel, split = flags >> 5, flags & 0x10
            ln = (struct.unpack(">H", d[p:p + 2])[0] + 7) // 8; p += 2
            if rel in (2, 3, 4, 6, 7): p += 3
            if rel in (1, 4): p += 3
            if rel in (1, 3, 4, 7): p += 4
            sp = None
            if split:
                sp = struct.unpack(">IHI", d[p:p + 10]); p += 10
            body = d[p:p + ln]; p += ln
            if sp:
                cnt, sid, idx = sp
                parts = self.splits.setdefault(sid, {})
                parts[idx] = body
                if len(parts) == cnt:
                    out.append(b"".join(parts[i] for i in range(cnt))); del self.splits[sid]
            else:
                out.append(body)
        return out

    def connect(self):
        self.s.settimeout(2)
        self.s.sendto(b"\x05" + MAGIC + b"\x0b" + b"\x00" * (1400 - 18), self.addr)
        d, _ = self.s.recvfrom(2048); assert d[0] == 0x06, d
        mtu = struct.unpack(">H", d[26:28])[0]
        port = self.addr[1]
        self.s.sendto(b"\x07" + MAGIC + b"\x04" + bytes([128, 255, 255, 254]) + struct.pack(">H", port)
                      + struct.pack(">H", mtu) + struct.pack(">Q", random.getrandbits(63)), self.addr)
        d, _ = self.s.recvfrom(2048); assert d[0] == 0x08, d
        self._frame(b"\x09" + struct.pack(">QQ", 42, 0) + b"\x00")
        for _ in range(5):
            if any(b[0] == 0x10 for b in self._recv(2)):
                break
        else:
            raise RuntimeError("no ConnectionRequestAccepted")
        self._frame(b"\x13" + b"\x04\x80\xff\xff\xfe" + struct.pack(">H", port) + b"\x00" * 7 * 20 + struct.pack(">QQ", 0, 0))

    # ---------------------------------------------------------------- game
    def send(self, *packets):
        raw = b"".join(uvarint(len(p)) + p for p in packets)
        if self.compression:
            c = zlib.compressobj(6, zlib.DEFLATED, -15)
            raw = b"\x00" + c.compress(raw) + c.flush()
        self._frame(b"\xfe" + raw)

    def _decode(self, body):
        raw = body[1:]
        if self.compression:
            raw = zlib.decompress(raw[1:], -15) if raw[0] == 0 else raw[1:]
        pks, p = [], 0
        while p < len(raw):
            n, p = read_uvarint(raw, p); pks.append(raw[p:p + n]); p += n
        return pks

    def pump(self, seconds, until=None):
        end = time.time() + seconds
        while time.time() < end:
            for body in self._recv(0.2):
                if body[0] != 0xfe:
                    continue
                for pk in self._decode(body):
                    pid, p = read_uvarint(pk, 0)
                    pid &= 0x3ff
                    self._handle(pid, pk[p:])
            if until and until():
                return True
        return False

    def _handle(self, pid, data):
        if pid == 0x3a:
            self.chunks += 1
            return
        self.received.append((pid, data))
        if pid == 0x8f:
            self.compression = True
            auth = json.dumps({"AuthenticationType": 2, "Certificate": "{}", "Token": ""}).encode()
            cd = jwt({"ThirdPartyName": self.name, "SkinId": "test", "SkinImageWidth": 64, "SkinImageHeight": 64,
                      "SkinData": base64.b64encode(bytes(64 * 64 * 4)).decode(), "DeviceOS": 8, "DeviceId": "bot"}).encode()
            blob = struct.pack("<I", len(auth)) + auth + struct.pack("<I", len(cd)) + cd
            self.send(uvarint(0x01) + struct.pack(">I", PROTOCOL) + uvarint(len(blob)) + blob)
        elif pid == 0x06:
            self.send(uvarint(0x08) + uvarint(2) + uvarint(0))  # have all packs
        elif pid == 0x07:
            self.send(uvarint(0x08) + uvarint(3) + uvarint(0))  # completed
        elif pid == 0x0b:
            self.send(uvarint(0x45) + zz(4) + b"\x04")
        elif pid == 0x02 and data[:4] == b"\x00\x00\x00\x03":
            self.send(uvarint(0x71) + uvarint(1))
            self.spawned = True

    def join(self, timeout=20):
        self.connect()
        self.send(uvarint(0xc1) + struct.pack(">I", PROTOCOL))
        if not self.pump(timeout, lambda: self.spawned):
            raise RuntimeError(f"{self.name} did not spawn")

    def move(self, x, y, z, pitch=0.0, yaw=0.0):
        self.pos = [x, y, z]
        body = struct.pack("<ff", pitch, yaw) + struct.pack("<fff", x, y, z) + struct.pack("<fff", 0, 0, yaw)
        self.send(uvarint(0x90) + body)

    def send_text(self, msg):
        self.send(uvarint(0x09) + b"\x00\x01\x01" + string(self.name) + string(msg) + string("") + string("") + b"\x00")

    def close(self):
        self.s.sendto(b"\x84" + struct.pack("<I", self.seq)[:3] + bytes([0]) + struct.pack(">H", 8) + b"\x15", self.addr)

    def ids(self):
        return [NAMES.get(pid, hex(pid)) for pid, _ in self.received]


def svarint(v):
    return zz(v)


def block_id(name):
    """Hashed network id of a block's default state, read from data/block_palette.nbt."""
    import gzip, os
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "data", "block_palette.nbt")
    d = gzip.open(path).read()
    needle = b"\x08\x00\x04name" + struct.pack(">H", len(name)) + name.encode()
    p = d.find(needle)
    q = d.rfind(b"\x03\x00\x0anetwork_id", 0, p)
    return struct.unpack(">i", d[q + 13:q + 17])[0]


def _block_pos(pos):
    return b"".join(svarint(v) for v in pos)


def _auth_input(c, block_actions=()):
    x, y, z = c.pos
    body = struct.pack("<ff", 0, 0) + struct.pack("<fff", x, y, z) + struct.pack("<fff", 0, 0, 0)
    body += uvarint(0) + uvarint(1) + uvarint(0) + svarint(0) + struct.pack("<ff", 0, 0)
    body += uvarint(1) + struct.pack("<fff", 0, 0, 0)
    body += b"\x00\x00"  # no item interaction, no item stack request
    if block_actions:
        body += b"\x01" + uvarint(len(block_actions))
        for action, pos, face in block_actions:
            body += svarint(action) + _block_pos(pos) + svarint(face)
    else:
        body += b"\x00"
    body += b"\x00\x00" + struct.pack("<ff", 0, 0) + struct.pack("<fff", 0, 0, 0) + struct.pack("<ff", 0, 0)
    return uvarint(0x90) + body


def break_block(c, pos):
    c.send(_auth_input(c, [(13, pos, 1)]))  # CREATIVE_PLAYER_DESTROY_BLOCK


def place_block(c, clicked, face, block):
    # an empty hand when no block is given (id 1 is a real item and would be placed)
    item = struct.pack("<hH", 1 if block else 0, 1 if block else 0) + uvarint(0) + b"\x00" + uvarint(block & 0xffffffff) + uvarint(0)
    data = svarint(0) + b"\x01" + _block_pos(clicked) + bytes([face]) + svarint(0) + b"\x00" + item
    data += struct.pack("<fff", *c.pos) + struct.pack("<fff", 0.5, 1.0, 0.5) + uvarint(0) + b"\x01\x00"
    c.send(uvarint(0x1e) + svarint(0) + b"\x00" + uvarint(2) + uvarint(0) + data)


Client.break_block = break_block
Client.place_block = place_block


def update_blocks(c):
    """[(pos, block id)] from every UpdateBlock received."""
    out = []
    for pid, d in c.received:
        if pid != 0x15:
            continue
        p, vals = 0, []
        for _ in range(3):
            v, p = read_uvarint(d, p); vals.append((v >> 1) ^ -(v & 1))
        bid, p = read_uvarint(d, p)
        out.append((tuple(vals), struct.unpack("<i", struct.pack("<I", bid))[0]))
    return out
