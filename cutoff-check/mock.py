import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
LOG = sys.argv[2]
REPLIES = [
    ("Plan first.\n\n### EDIT: a.txt\n<<<<<<< SEARCH\ntwo\n=======\nTWO\n>>>>>>> REPLACE\n\n### FILE: b.jsx\n```jsx\nline 1\nline 2\nline 3\nli", "incomplete"),
    ("### APPEND: b.jsx\n```jsx\nline 3\nline 4\n```\n\nDone.", "completed"),
]
n = 0
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        global n
        body = self.rfile.read(int(self.headers.get('content-length', 0)))
        with open(LOG, 'a') as f:
            f.write(json.dumps({"path": self.path, "body": json.loads(body or b'{}')}) + "\n")
        text, kind = REPLIES[min(n, len(REPLIES) - 1)]
        rid = f"resp_{n}"
        n += 1
        self.send_response(200)
        self.send_header('content-type', 'text/event-stream')
        self.end_headers()
        def ev(o): self.wfile.write(b"data: " + json.dumps(o).encode() + b"\n\n")
        ev({"type": "response.created", "response": {"id": rid}})
        for i in range(0, len(text), 20):
            ev({"type": "response.output_text.delta", "delta": text[i:i+20]})
        resp = {"id": rid, "usage": {"input_tokens": 100, "output_tokens": 50}, "output": []}
        if kind == "incomplete":
            resp["incomplete_details"] = {"reason": "max_output_tokens"}
        ev({"type": f"response.{kind}", "response": resp})
        self.wfile.write(b"data: [DONE]\n\n")
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
