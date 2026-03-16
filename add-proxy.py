import re

# Add proxy routes to server.js for mesh node stats
server_path = '/home/opc/kannaka-radio/server.js'
content = open(server_path).read()

# Add proxy route before the mesh route
proxy_code = '''
  // ── Mesh proxy — avoid mixed-content by proxying node stats ──
  if (parsed.pathname.startsWith("/api/mesh/")) {
    const targets = {
      "node1": "http://170.9.238.136:9001",
      "node2": "http://170.9.241.14:9001",
      "node3": "http://localhost:9001"
    };
    const parts = parsed.pathname.split("/");
    const nodeKey = parts[3]; // e.g. "node1"
    const endpoint = "/" + (parts.slice(4).join("/") || "status");
    const target = targets[nodeKey];
    if (!target) {
      res.writeHead(404, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "unknown node" }));
      return;
    }
    try {
      const url = target + endpoint;
      const controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), 3000);
      const resp = await fetch(url, { signal: controller.signal });
      clearTimeout(timeout);
      const body = await resp.text();
      res.writeHead(200, { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" });
      res.end(body);
    } catch (e) {
      res.writeHead(502, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "unreachable", detail: e.message }));
    }
    return;
  }

'''

# Insert before the mesh route
content = content.replace('  if (parsed.pathname === "/mesh")', proxy_code + '  if (parsed.pathname === "/mesh")')

# Make the request handler async if it isn't already
# Check if createServer callback is async
if 'async function' not in content.split('createServer')[1][:200] if 'createServer' in content else True:
    content = content.replace('const server = http.createServer((req, res) => {', 'const server = http.createServer(async (req, res) => {')
    content = content.replace('const server = http.createServer(function(req, res) {', 'const server = http.createServer(async function(req, res) {')

open(server_path, 'w').write(content)

# Now update mesh.html to use proxy URLs
mesh_path = '/home/opc/kannaka-radio/mesh.html'
mesh = open(mesh_path).read()
mesh = mesh.replace('"http://170.9.238.136:9001/status"', '"/api/mesh/node1/status"')
mesh = mesh.replace('"http://170.9.241.14:9001/status"', '"/api/mesh/node2/status"')
mesh = mesh.replace('"http://localhost:9001/status"', '"/api/mesh/node3/status"')
open(mesh_path, 'w').write(mesh)

print("Done — proxy routes added, mesh.html updated to use relative URLs")
