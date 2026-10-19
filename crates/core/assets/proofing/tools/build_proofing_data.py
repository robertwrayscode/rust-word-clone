#!/usr/bin/env python3
"""Regenerate RWord's bundled proofing data (words.bin, thesaurus.bin).

Usage (paths to the downloaded sources, see ../SOURCES.md):

    python3 -I build_proofing_data.py \
        --us-aff en_US.aff --us-dic en_US.dic \
        --gb-aff dictionary-en-gb/index.aff --gb-dic dictionary-en-gb/index.dic \
        --scowl scowl-2020.12.07 \
        --thes th_en_US_v2.dat \
        --extra ../extra_words.txt \
        --out ..

The script only prints counts; it never prints word-list contents.

words.bin  = zlib( front-coded sorted word list )
  each entry: <prefix-len byte> <suffix UTF-8 bytes> <class byte>
  class byte is 0x01 + class, class 0..=6 = SCOWL size bucket
  (10, 20, 35, 40, 50, 55, >=60/unknown) and 7 = NOSUGGEST (accepted when
  typed, never offered as a suggestion or synonym).

thesaurus.bin = zlib( lines sorted by head word, '\n' separated )
  line: head \x1f pos \x1e syn \x1e syn ... [\x1f pos \x1e syn ...]
"""
import argparse
import re
import zlib

SIZE_BUCKETS = [10, 20, 35, 40, 50, 55]
NOSUGGEST = 7
UNKNOWN = 6
MAX_SYNS = 12
MAX_MEANINGS = 8


def load_aff(path):
    rules = {}
    lines = open(path, encoding='utf-8').read().splitlines()
    i = 0
    while i < len(lines):
        p = lines[i].split()
        if len(p) == 4 and p[0] in ('PFX', 'SFX') and p[2] in 'YN':
            kind, flag, cross, n = p[0], p[1], p[2] == 'Y', int(p[3])
            lst = []
            for j in range(n):
                q = lines[i + 1 + j].split()
                strip = '' if q[2] == '0' else q[2]
                add = '' if q[3] == '0' else q[3].split('/')[0]
                cond = q[4] if len(q) > 4 else '.'
                rx = re.compile(('^' + cond) if kind == 'PFX' else (cond + '$'))
                lst.append((strip, add, rx))
            rules[flag] = (kind, cross, lst)
            i += n + 1
        else:
            i += 1
    return rules


def expand(dic, rules, out, nosug):
    lines = open(dic, encoding='utf-8-sig').read().splitlines()[1:]
    for line in lines:
        line = line.strip()
        if not line:
            continue
        w, flags = (line.split('/', 1) + [''])[:2]
        flags = flags.split()[0] if flags else ''
        if 'c' in flags or any(ch.isdigit() for ch in w):
            continue
        target = nosug if '!' in flags else out
        forms = [w]
        sfx_forms = []
        for f in flags:
            r = rules.get(f)
            if not r or r[0] != 'SFX':
                continue
            for strip, add, rx in r[2]:
                if rx.search(w) and (not strip or w.endswith(strip)):
                    nw = (w[:len(w) - len(strip)] if strip else w) + add
                    forms.append(nw)
                    if r[1]:
                        sfx_forms.append(nw)
        for f in flags:
            r = rules.get(f)
            if not r or r[0] != 'PFX':
                continue
            for strip, add, rx in r[2]:
                for base in [w] + (sfx_forms if r[1] else []):
                    if rx.search(base):
                        forms.append(add + base[len(strip):])
        target.update(forms)


def scowl_sizes(root):
    import os
    sizes = {}
    final = os.path.join(root, 'final')
    for name in os.listdir(final):
        m = re.match(r'^(english|american|british|british_z|canadian)-'
                     r'(words|upper|proper-names|contractions|abbreviations)\.(\d+)$', name)
        if not m:
            continue
        size = int(m.group(3))
        for w in open(os.path.join(final, name), encoding='latin-1').read().split('\n'):
            w = w.strip()
            if w and (w not in sizes or sizes[w] > size):
                sizes[w] = size
    return sizes


def taboo(root):
    import os
    lists = {}
    for name in ('offensive.1', 'offensive.2', 'profane.1', 'profane.3'):
        p = os.path.join(root, 'misc', name)
        lists[name] = {w.strip() for w in open(p, encoding='latin-1') if w.strip()}
    return lists


def bucket(size):
    if size is None:
        return UNKNOWN
    for i, b in enumerate(SIZE_BUCKETS):
        if size <= b:
            return i
    return UNKNOWN


def main():
    ap = argparse.ArgumentParser()
    for a in ('us-aff', 'us-dic', 'gb-aff', 'gb-dic', 'scowl', 'thes', 'extra', 'out'):
        ap.add_argument('--' + a, required=True)
    a = ap.parse_args()

    words, nosug = set(), set()
    expand(a.us_dic, load_aff(a.us_aff), words, nosug)
    expand(a.gb_dic, load_aff(a.gb_aff), words, nosug)
    print('hunspell words', len(words), 'nosuggest', len(nosug))

    extra = [x for l in open(a.extra, encoding='utf-8') if not l.startswith('#') for x in l.split()]
    new_extra = [w for w in extra if w not in words]
    print('extra words', len(extra), 'new', len(new_extra))

    sizes = scowl_sizes(a.scowl)
    lists = taboo(a.scowl)
    strong = lists['offensive.1'] | lists['offensive.2'] | lists['profane.1']
    mild = {w for w in lists['profane.3'] if (sizes.get(w) or 99) > 35}
    taboo_base = {w.lower() for w in strong | mild}
    # Inflected forms of taboo words that exist in the list are NOSUGGEST too.
    for w in list(words):
        lw = w.lower()
        for base in taboo_base:
            if lw == base or (lw.startswith(base) and lw[len(base):] in
                              ("s", "es", "ed", "er", "ers", "ing", "y", "'s", "s'", "ier", "iest")):
                nosug.add(w)
                break
    words -= nosug
    print('nosuggest total', len(nosug))

    cls = {}
    for w in words:
        cls[w] = bucket(sizes.get(w))
    for w in new_extra:
        cls[w] = 2
    for w in nosug:
        cls[w] = NOSUGGEST
    hist = [0] * 8
    for c in cls.values():
        hist[c] += 1
    print('class histogram', hist)

    # Front-coded word list.
    buf = bytearray()
    prev = b''
    for w in sorted(cls, key=lambda s: s.encode('utf-8')):
        b = w.encode('utf-8')
        assert all(x >= 0x20 for x in b)
        n = 0
        while n < min(len(prev), len(b), 255) and prev[n] == b[n]:
            n += 1
        buf.append(n)
        buf += b[n:]
        buf.append(1 + cls[w])
        prev = b
    comp = zlib.compress(bytes(buf), 9)
    open(a.out + '/words.bin', 'wb').write(comp)
    print('words', len(cls), 'raw', len(buf), 'compressed', len(comp))

    # Thesaurus.
    bad = {w.lower() for w in nosug}

    def known(t):
        # Mirrors Speller::lookup: exact, or Title/UPPER case of a lower-case word.
        if t.lower() in bad:
            return False
        if cls.get(t, NOSUGGEST) != NOSUGGEST:
            return True
        return (t[:1].isupper() and t[1:].islower() or t.isupper()) and cls.get(t.lower(), NOSUGGEST) != NOSUGGEST

    def ok_phrase(p):
        return all(known(t) for t in re.split(r'[ \-]', p) if t)

    lines = open(a.thes, encoding='utf-8').read().splitlines()
    i = 1
    out_lines = []
    while i < len(lines):
        head, n = lines[i].rsplit('|', 1)
        n = int(n)
        ms = lines[i + 1:i + 1 + n]
        i += 1 + n
        if ' ' in head or head not in cls or cls[head] == NOSUGGEST:
            continue
        if cls[head] > 4 and head not in new_extra:  # keep SCOWL size <= 50
            continue
        meanings = []
        for m in ms:
            parts = m.split('|')
            pos = parts[0].strip('()')
            syns = []
            for s in parts[1:]:
                if '(antonym)' in s:
                    continue
                s = re.sub(r'\s*\([^)]*\)\s*$', '', s).strip()
                if not s or s.lower() == head.lower() or s in syns or not ok_phrase(s):
                    continue
                syns.append(s)
                if len(syns) >= MAX_SYNS:
                    break
            if syns:
                meanings.append(pos + '\x1e' + '\x1e'.join(syns))
            if len(meanings) >= MAX_MEANINGS:
                break
        if meanings:
            out_lines.append(head + '\x1f' + '\x1f'.join(meanings))
    out_lines.sort(key=lambda s: s.split('\x1f', 1)[0].encode('utf-8'))
    raw = '\n'.join(out_lines).encode('utf-8')
    comp = zlib.compress(raw, 9)
    open(a.out + '/thesaurus.bin', 'wb').write(comp)
    print('thesaurus heads', len(out_lines), 'raw', len(raw), 'compressed', len(comp))


if __name__ == '__main__':
    main()
