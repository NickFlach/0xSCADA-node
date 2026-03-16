content = open('/home/opc/kannaka-radio/workspace/index.html').read()

# Remove the broken Ctrl+Enter block
bad = '''// Ctrl+Enter — mesh dashboard backdoor
document.addEventListener(" keydown, function(e) {
 if (e.ctrlKey && e.key === Enter) {
 e.preventDefault();
 window.location.href = /mesh;
 }
});
'''
content = content.replace(bad, '')

# Also try alternate broken versions
import re
content = re.sub(r'// Ctrl\+Enter.*?}\);?\n', '', content, flags=re.DOTALL)

# Insert the correct version before the Konami Code block
good = '''// Ctrl+Enter — mesh dashboard backdoor
document.addEventListener("keydown", function(e) {
  if (e.ctrlKey && e.key === "Enter") {
    e.preventDefault();
    window.location.href = "/mesh";
  }
});

'''
content = content.replace('// Konami Code\n', good + '// Konami Code\n')

open('/home/opc/kannaka-radio/workspace/index.html', 'w').write(content)
print("Fixed Ctrl+Enter handler")
