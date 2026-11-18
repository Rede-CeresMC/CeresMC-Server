"""Resource packs (python tools/packs_test.py): the server offers the pack in resource_packs/, the
client downloads it chunk by chunk, checks its SHA-256 and gets it in the pack stack."""
import sys, os, struct, hashlib
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, uvarint, read_uvarint, string

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def read_string(d, p):
    n, p = read_uvarint(d, p)
    return d[p:p + n], p + n


class PackClient(Client):
    """Downloads every pack instead of saying it has them all."""
    def __init__(self, name):
        super().__init__(name)
        self.offered = []      # (uuid, version, size, must_accept)
        self.info = {}         # uuid -> (chunk count, size, sha256)
        self.data = {}         # uuid -> {index: bytes}
        self.stack = None
        self.chunk_offsets_ok = True

    def _handle(self, pid, d):
        if pid == 0x06:
            self.received.append((pid, d))
            must = d[0] == 1
            p = 4 + 16
            _, p = read_string(d, p)          # world template version
            n, p = read_uvarint(d, p)
            for _ in range(n):
                p += 16                        # uuid bytes
                version, p = read_string(d, p)
                size = struct.unpack_from("<Q", d, p)[0]; p += 8
                for _ in range(3):
                    _, p = read_string(d, p)   # key, sub pack, content id
                content_id = d[p - 36:p].decode()
                p += 3
                _, p = read_string(d, p)       # cdn url
                self.offered.append((content_id, version.decode(), size, must))
            if self.offered:
                ids = [f"{u}_{v}" for u, v, _, _ in self.offered]
                self.send(uvarint(0x08) + uvarint(1) + string("") + uvarint(len(ids)) + b"".join(string(i) for i in ids))
            else:
                self.send(uvarint(0x08) + uvarint(2) + uvarint(0))
            return
        if pid == 0x52:
            uuid, p = read_string(d, 0)
            uuid = uuid.decode()
            chunk_size, count = struct.unpack_from("<II", d, p); p += 8
            size = struct.unpack_from("<Q", d, p)[0]; p += 8
            sha, p = read_string(d, p)
            self.info[uuid] = (count, size, sha, chunk_size)
            self.data[uuid] = {}
            self.send(uvarint(0x54) + string(uuid) + struct.pack("<I", 0))
            return
        if pid == 0x53:
            uuid, p = read_string(d, 0)
            uuid = uuid.decode()
            index = struct.unpack_from("<I", d, p)[0]; p += 4
            offset = struct.unpack_from("<Q", d, p)[0]; p += 8
            chunk, p = read_string(d, p)
            count, _, _, chunk_size = self.info[uuid]
            self.chunk_offsets_ok &= offset == index * chunk_size
            self.data[uuid][index] = chunk
            if index + 1 < count:
                self.send(uvarint(0x54) + string(uuid) + struct.pack("<I", index + 1))
            elif all(len(self.data[u]) == self.info[u][0] for u in self.info):
                self.send(uvarint(0x08) + uvarint(2) + uvarint(0))  # have all packs now
            return
        if pid == 0x07:
            p = 1
            n, p = read_uvarint(d, p)
            self.stack = []
            for _ in range(n):
                u, p = read_string(d, p)
                v, p = read_string(d, p)
                _, p = read_string(d, p)
                self.stack.append((u.decode(), v.decode()))
        super()._handle(pid, d)


c = PackClient("BotPacks")
c.join(timeout=60)
check(len(c.offered) == 1, f"the server offers the test pack ({[(u, v) for u, v, _, _ in c.offered]})")
if c.offered:
    uuid, version, size, must = c.offered[0]
    check(must, "the pack must be accepted (force-resource-packs)")
    check(version == "1.0.0", f"its version comes from manifest.json ({version})")
    count, info_size, sha, _ = c.info.get(uuid, (0, 0, b"", 0))
    check(count >= 2, f"it is sent in several chunks ({count})")
    data = b"".join(c.data.get(uuid, {}).get(i, b"") for i in range(count))
    check(len(data) == size == info_size, f"all of it arrives ({len(data)} of {size} bytes)")
    check(hashlib.sha256(data).digest() == sha, "the SHA-256 matches")
    check(c.chunk_offsets_ok, "each chunk says its offset")
    check(data[:4] == b"PK\x03\x04", "the folder pack is sent as a zip")
    check(c.stack is not None and (uuid, version) in c.stack, f"the pack is in the stack ({c.stack})")
check(c.spawned, "after the download the player joins")

# asking for a chunk twice is refused (the session is closed)
c2 = PackClient("BotPacks2")
orig = c2._handle
def twice(pid, d):
    if pid == 0x52:
        uuid, _ = read_string(d, 0)
        c2.send(uvarint(0x54) + string(uuid.decode()) + struct.pack("<I", 0))
        c2.send(uvarint(0x54) + string(uuid.decode()) + struct.pack("<I", 0))
        c2.received.append((pid, d))
        return
    orig(pid, d)
c2._handle = twice
try:
    c2.join(timeout=8)
    joined = True
except Exception:
    joined = False
check(not joined, "asking for the same chunk twice is refused")

print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
