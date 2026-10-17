# ADR-0005: source-preserving media placement

Status: accepted, 2026-10-09.

Move remains the default. Copy, hardlink and file symlink are distinct operations, never aliases for MoveFile. Retained operations carry the preview source fingerprint, preserve videos and companion subtitles, and suppress source-directory trash cleanup in core.

Journal v4 extends durable v2 staging/proof ownership and v3 independent metadata replacements. Intent is persisted before file creation. Publication is exclusive, with the staging inode retained as ownership evidence. Undo validates digest and inode before deleting a copy/hardlink target; a symlink target is removed only after link inode/text verification. Hardlink cross-volume errors are explicit and never become a copy. Shared source/hardlink inodes are never chmodded. A newer sidecar still referencing media blocks older undo.

`kiroku.json` placements are disk truth. A registered link must also match its v4 journal operation and staging anchor; an arbitrary symlink or forged manifest alone cannot opt into link support. Only ordinary media/subtitle files are supported; directory links and management-file targets remain forbidden. Library index, append, playback and health share the same resolver. Source loss or replacement prevents playback and aborts reindex without deleting the cached entry.

Revalidation updates only source proofs via an explicit metadata revision preview and save, with source fingerprints checked again during publication. It cannot accept a replaced link inode. Maintenance protects referenced anchors and does not traverse or remove outside sources. Logical linked media size is distinct from conservative reclaimable-byte estimates.

Compatibility: old settings select move; old sidecars default to no placements; journal v1–v3 remain readable/recoverable. Keep v4 journals and anchors while a sidecar references them. Never manually purge active `.kiroku-*.tmp` files.
