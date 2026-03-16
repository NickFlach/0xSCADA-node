lines = open('/home/opc/kannaka-radio/mesh.html').readlines()
lines[83] = '  { name: "LAPTOP", url: "http://localhost:9001/status", label: "local-3" }\n'
# Also remove any LOCAL ONLY references
content = ''.join(lines).replace('LOCAL ONLY', 'LOCAL')
open('/home/opc/kannaka-radio/mesh.html', 'w').write(content)
print("Fixed line 84:", lines[83].strip())
