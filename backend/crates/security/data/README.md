# Common password blocklist

`pwdb-top-100000.txt` is the top 100,000 entries of the Pwdb leaked-password
frequency list, used for the offline password policy (spec §8.2).

- Source: <https://github.com/danielmiessler/SecLists/blob/12274c98fdebe98c7a7284914436a472ed469aed/Passwords/Common-Credentials/Pwdb_top-100000.txt>
- Upstream commit: `12274c98fdebe98c7a7284914436a472ed469aed` (2025-03-22)
- License: MIT (SecLists, Copyright (c) 2018 Daniel Miessler)
- Format: one password per line, UTF-8, exactly 100,000 lines

The list is embedded into the binary at build time with `include_str!`; it is
never fetched at runtime. Matching happens after NFC normalization and
lowercasing, so the list is compared case-insensitively. The list is versioned
by updating this file and the commit above in the same change.
