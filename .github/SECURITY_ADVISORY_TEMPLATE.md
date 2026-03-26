# Security Advisory Template

> This file is a guide for maintainers when drafting a GitHub Security Advisory.
> Go to **Security → Advisories → New draft advisory** to create one.

---

## Advisory Fields

### Ecosystem
`Rust` / `crates.io` (for dependency issues) or `Other` for operator-level issues.

### Package Name
`stellar-k8s`

### Affected Versions
Use a range, e.g.: `>= 0.1.0, < 0.1.1`

### Patched Version
e.g.: `0.1.1`

### Severity
Assign using [CVSS v3.1 calculator](https://www.first.org/cvss/calculator/3.1):

| Rating   | CVSS Score |
| -------- | ---------- |
| Critical | 9.0–10.0   |
| High     | 7.0–8.9    |
| Medium   | 4.0–6.9    |
| Low      | 0.1–3.9    |

---

## Advisory Body Template

```markdown
## Summary

Brief one-paragraph description of the vulnerability.

## Details

Detailed technical description:
- Affected component(s)
- Root cause
- Attack vector and conditions required

## Impact

What can an attacker achieve? Who is affected?

## Patches

Fixed in version X.Y.Z. Users should upgrade immediately.

Commit: <link to fix commit>

## Workarounds

If no patch is available yet, describe any mitigations.

## References

- CVE: CVE-YYYY-NNNNN (if assigned)
- Related issue: #<issue number> (if public)
- RUSTSEC advisory: RUSTSEC-YYYY-NNNN (if applicable)

## Credits

Reported by [Reporter Name / Handle] via responsible disclosure.
```

---

## CVE Assignment

For Critical/High severity issues, request a CVE via:
- [GitHub's CVE numbering authority](https://github.com/github/advisory-database) (automatic when publishing a GitHub advisory)
- Or directly via [MITRE CVE Request](https://cveform.mitre.org/)

---

## Checklist Before Publishing

- [ ] Fix is merged and released
- [ ] Patched version is tagged and published
- [ ] CHANGELOG.md updated with security notice
- [ ] CVE assigned (if warranted)
- [ ] Reporter credited (or confirmed they want anonymity)
- [ ] Advisory reviewed by at least one other maintainer
