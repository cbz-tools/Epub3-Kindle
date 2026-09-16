# Authority-first conversion audit E2E

This suite is intentionally independent from the production serializer/parser implementation.

## Rules

1. Requirements come from the authorities documented in `docs/CONVERSION_AUDIT.md` and the frozen product contract.
2. Input recipes in `audit_support/epub.rs` are authored from those requirements, not copied from the former E2E fixtures.
3. Generated AZW3/MOBI bytes are inspected by `audit_support/palm.rs`, which implements only the independently documented PalmDB/PalmDOC/MOBI/EXTH observations needed by the audit.
4. Audit code must not import private production `epub`, `kindle`, `kf8`, `mobi`, or `container` modules, production constants, or production serializer helpers.
5. A test passing does not automatically make every linked requirement PASS. The requirement's complete declared input/assertion scope must have fresh evidence.
6. Reference-, physical-device-, and real-corpus requirements stay PENDING/GAP until their own evidence is collected.

## Corpus classification

- Deterministic fixtures are a generic/synthetic EPUB 3 compatibility corpus covering structure, navigation, CSS, resources and fonts, writing mode and layout, KF8/Dual MOBI serialization, and large geometry.
- Producer-generated EPUBs are interoperability sources. AozoraEpub3-generated assets retain that provenance without defining the product's input identity.
- KindleGen is a compatibility and reference implementation for same-input comparisons.
- Documentation, physical-device, and demo assets are tracked separately from the deterministic compatibility corpus.

The former audit/E2E may be consulted only after this specification is frozen, and only as an omission checklist. A former test or implementation behavior is never itself a requirement authority.
