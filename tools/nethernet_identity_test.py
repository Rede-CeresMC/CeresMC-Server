"""NetherNet signalling with xbox-auth=on: offers without a client identity, or with one that does
not match the offer's DTLS fingerprint, are refused before any WebRTC work starts.
(python tools/nethernet_identity_test.py <server log>)"""
import sys, os, time, base64, json, http.client

port = int(os.environ.get("CERES_PORT", "19132"))
log = sys.argv[1]
ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def post(sdp):
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    conn.request("POST", "/v1/join/12345", body=sdp.encode(), headers={"Content-Type": "application/sdp"})
    status = conn.getresponse().status
    conn.close()
    return status

def log_has(text):
    time.sleep(0.3)
    return text in open(log, encoding="utf-8", errors="replace").read()

base = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=fingerprint:sha-256 AA:BB\r\n"
check(post(base) == 503 and log_has("carries no identity"), "an offer without identity is refused")

def b64url(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()
token = ".".join([b64url(b'{"alg":"RS256"}'), b64url(b'{"cpk":"AAAA","exp":9999999999}'), "sig"])
assertion = json.dumps({"fingerprints": b64url(b'{"alg":"ES384"}') + "..sig", "token": token})
ident = base64.b64encode(json.dumps({"assertion": assertion, "idp": {"domain": "x", "protocol": "default"}}).encode()).decode()
check(post(base + f"a=identity:{ident}\r\n") == 503 and log_has("invalid identity"), "a forged identity is refused")

print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
