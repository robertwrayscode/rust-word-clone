# Proofing data: sources and licences

The files in this directory are bundled into `wordcore` with `include_bytes!`
and decoded on first use by `crates/core/src/proofing/`.

| File | Contents | Built from |
|------|----------|-----------|
| `words.bin` | 126,898 English words (US + British spellings), front-coded and deflated (~206 KB) | Hunspell en_US 2020.12.07 (SCOWL size 60), Hunspell en_GB-ise 2020.12.07 (SCOWL size 60, packaged as npm `dictionary-en-gb` 3.0.0), SCOWL 2020.12.07 size lists (for word commonness), SCOWL `misc/` taboo lists (to mark words NOSUGGEST), `extra_words.txt` |
| `thesaurus.bin` | 32,308 head words with meanings and synonyms, deflated (~1 MB) | LibreOffice/OpenOffice MyThes thesaurus `th_en_US_v2.dat` (derived from Princeton WordNet) |
| `extra_words.txt` | ~290 modern words (email, smartphone, iPhone, Bluetooth …) | written for RWord (MIT, like the rest of RWord) |

Regenerate with `tools/build_proofing_data.py` (see its docstring for the
command line). The script only prints counts.

## Processing

* Hunspell `.dic`/`.aff` files are expanded to full word forms (prefixes and
  suffixes, cross products). Number-compounding entries are dropped (the
  checker ignores words containing digits).
* Each word gets a commonness class from the smallest SCOWL size list that
  contains it (10, 20, 35, 40, 50, 55, ≥60). Suggestions prefer common words.
* Words flagged NOSUGGEST in the Hunspell dictionaries, plus the words in
  SCOWL's `misc/offensive.1`, `offensive.2`, `profane.1` and the uncommon
  entries of `profane.3` (and their inflections), are stored with class
  NOSUGGEST: they are accepted when typed but never offered as spelling
  suggestions, and they are removed from the thesaurus (both as head words
  and as synonyms).
* Thesaurus: only single-word heads that are in the word list with SCOWL size
  ≤ 50 are kept; antonyms are dropped; synonyms must consist of dictionary
  words; at most 8 meanings per word and 12 synonyms per meaning.

## Licences

All sources are under permissive licences that allow redistribution in a
binary with the copyright notices retained:

* SCOWL and the Hunspell en_US / en_GB dictionaries derived from it —
  copyright Kevin Atkinson and others, MIT-like / BSD-like terms:
  `licenses/SCOWL-Copyright.txt`, `licenses/SCOWL-README.txt`,
  `licenses/hunspell-en_US-README.txt`, `licenses/dictionary-en-gb-LICENSE.txt`,
  `licenses/dictionary-en-gb-README.md`.
* WordNet (the basis of the MyThes thesaurus) — Princeton University WordNet
  licence: `licenses/WordNet-LICENSE.txt`.

The application's About box / third-party notices should reproduce these
notices.
