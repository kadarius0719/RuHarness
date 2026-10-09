import re, sys
path = sys.argv[1]
lines = open(path, encoding='utf-8').read().split('\n')
fail = 0
# 1. fences
infence = None; fences = []
for i, l in enumerate(lines, 1):
    m = re.match(r'^\s*(`{3,})(.*)$', l)
    if infence is None:
        if m:
            infence = (i, len(m.group(1)))
    else:
        if m and len(m.group(1)) >= infence[1] and m.group(2).strip() == '':
            fences.append((infence[0], i)); infence = None
if infence:
    print('UNCLOSED fence at', infence[0]); fail += 1
print('fences:', len(fences))
# 2. every box labelled (the nearest non-blank line before it)
# A box's label is the nearest of these above it (a later box of the same
# label, such as the second of "one of these two lines", shares it); a
# heading, "What it means" or "If you do not see that" met first means the
# box has no label of its own.
LABELS = ('**Run', '**Do', 'You should see', 'you should see', 'The whole screen')
STOPS = ('**What it means', '**If you do not see that', '#')
nolabel = []
for o, c in fences:
    j = o - 2
    found = None
    while j >= 0:
        l = lines[j]
        inbox = any(oo <= j + 1 <= cc for oo, cc in fences)
        if not inbox and l.strip():
            if any(x in l for x in LABELS):
                found = 'ok'; break
            if any(l.startswith(x) for x in STOPS):
                found = l; break
        j -= 1
    if found != 'ok':
        nolabel.append((o, (found or '')[:100]))
for o, p in nolabel:
    print('NO LABEL box at', o, '| prev:', p)
fail += len(nolabel)
# 3. steps
def infen(i):
    return any(o <= i <= c for o, c in fences)
heads = [(i, l) for i, l in enumerate(lines, 1) if l.startswith('#') and not infen(i)]
stepids = set()
for idx, (i, l) in enumerate(heads):
    m = re.match(r'^#{3,4} Step ([0-9B]+\.[0-9]+)\b', l)
    if not m:
        continue
    stepids.add(m.group(1))
    end = heads[idx + 1][0] if idx + 1 < len(heads) else len(lines)
    body = '\n'.join(lines[i:end - 1])
    miss = []
    if '**Run' not in body and '**Do' not in body: miss.append('Run/Do')
    if 'You should see' not in body: miss.append('You should see')
    if '**What it means' not in body: miss.append('What it means')
    if '**If you do not see that' not in body: miss.append('If you do not see that')
    if miss:
        print('STEP', m.group(1), 'line', i, 'missing', miss); fail += 1
print('steps:', len(stepids))
# 4. parts
parts = sum(1 for l in lines if l.startswith('## Part'))
print('## Part count:', parts)
if parts != 13:
    fail += 1
# 5. step references
text = '\n'.join(lines)
bad = set(); nrefs = 0
pat = r'Steps?\s+((?:[0-9B]+\.[0-9]+)(?:(?:\s*(?:–|-|,|and|or|to|then)\s*)(?:[0-9B]+\.[0-9]+))*)'
for m in re.finditer(pat, text):
    for ref in re.findall(r'[0-9B]+\.[0-9]+', m.group(1)):
        nrefs += 1
        if ref not in stepids:
            ln = text[:m.start()].count('\n') + 1
            bad.add((ln, ref))
for ln, ref in sorted(bad):
    print('BAD REF Step', ref, 'at line', ln); fail += 1
print('step references checked:', nrefs)
# 6. internal links
anchors = set()
for i, l in heads:
    t = l.lstrip('#').strip().lower()
    t = re.sub(r'[^\w\- ]', '', t).replace(' ', '-')
    anchors.add(t)
for m in re.finditer(r'\]\(#([^)]+)\)', text):
    if m.group(1) not in anchors:
        ln = text[:m.start()].count('\n') + 1
        print('BAD LINK #' + m.group(1), 'at line', ln); fail += 1
print('FAIL' if fail else 'ALL GATES PASS', fail)
print('lines:', len(lines) - (1 if lines[-1] == '' else 0))
