# Feature: Vault Segmentation

## Overview

Vault Segmentation allows PocketVault to divide encrypted vault data into multiple `.pv` segment files instead of storing the entire vault as a single file.

The default segment size is **5 GiB**.

For example, a 350 GiB vault may be stored as:

```text
MyVault/
├── vault 1.5.pv
├── vault 2.10.pv
├── vault 3.15.pv
├── ...
└── vault 70.350.pv
```

Each segment contains multiple encrypted chunks. From the user's perspective, these segments still represent **one logical vault**.

---

# Motivation

PocketVault is designed for long-term storage of personal data on physical storage devices such as HDDs, SSDs, and USB drives.

Large vaults may eventually reach:

* 100 GB
* 500 GB
* 1 TB
* 2 TB+

Storing an entire vault as one file makes large-scale data management inconvenient.

For example:

```text
vault.pv
└── 2 TB
```

A single 2 TB file is difficult to visually inspect, migrate, and manage through a normal filesystem.

At the other extreme, storing every encrypted chunk as an individual file can result in thousands or hundreds of thousands of files:

```text
chunk-000001
chunk-000002
chunk-000003
...
```

This creates unnecessary filesystem overhead and makes manual migration difficult.

**Segmentation provides a middle ground:**

```text
One huge file
       ↕
   Segmented vault
       ↕
Thousands of small files
```

The goal is not primarily to optimize computer performance.

The goal is to make **large encrypted data easier for humans to manage**.

---

# Design Goals

## 1. Reduce the number of physical files

Instead of exposing every encrypted chunk as a separate filesystem object, PocketVault groups chunks into larger `.pv` segments.

For example:

```text
100,000 encrypted chunks
        ↓
20 × 5 GiB segments
```

This keeps the filesystem manageable while preserving chunk-level encryption internally.

---

## 2. Make migration easier to inspect

Segmentation provides a simple, human-readable representation of a vault.

For example:

```text
vault 1.5.pv
vault 2.10.pv
vault 3.15.pv
...
vault 69.345.pv
vault 70.350.pv
```

The user can immediately see:

* There are 70 segments.
* The vault contains approximately 350 GiB of logical data.
* The segments are sequential.

If a segment is missing during migration:

```text
vault 59.295.pv
vault 61.305.pv
```

the missing `vault 60.300.pv` can be identified visually.

This is only a **human-level sanity check**. Cryptographic integrity verification remains the authoritative mechanism for determining whether the vault is valid.

---

## 3. Separate logical storage from physical storage

The encryption/chunking layer and the physical segment layout should remain separate.

```text
Logical file
    ↓
Chunk
    ↓
Encrypt
    ↓
Encrypted chunk
    ↓
Segment writer
    ↓
.pv segment
```

Therefore:

* **Chunk** = logical/encryption unit
* **Segment** = physical storage unit

Changing segment size should not require changing the encryption model.

If existing ciphertext can be reused, repacking should move ciphertext rather than decrypting and encrypting the data again.

---

# Default Segment Size

The default segment size is:

**5 GiB**

5 GiB is intended as a balanced default rather than a technically optimal or mandatory value.

Example:

| Vault Size | 5 GiB Segments |
| ---------: | -------------: |
|    100 GiB |             20 |
|    500 GiB |            100 |
|      1 TiB |            205 |
|      2 TiB |            410 |

The exact number of segments depends on the actual logical vault size and the final segment.

The format should not be hard-coded around 5 GiB.

---

# Segment Setting

Add the following setting:

**Storage → Segment Size**

Possible values may include:

```text
1 GiB
5 GiB   ← Default
10 GiB
20 GiB
Custom
Single File
```

The UI should show an estimate before applying the change.

Example:

```text
Vault size: 350 GiB

Segment size: 5 GiB
Estimated segments: 70
```

Changing to 10 GiB:

```text
Segment size: 10 GiB
Estimated segments: 35
```

This allows users to choose a layout based on their current storage or migration requirements.

---

# Segment Naming

The default naming convention is:

```text
vault <segment_index>.<cumulative_size>.pv
```

For example:

```text
vault 1.5.pv
vault 2.10.pv
vault 3.15.pv
...
vault 60.300.pv
vault 70.350.pv
```

Where:

* `1`, `2`, `3`, etc. represent the segment index.
* `5`, `10`, `15`, etc. represent the cumulative logical data size after that segment.

The naming convention is intentionally human-readable to make large vaults easier to inspect and migrate.

### Filename is not the source of truth

The filename must not be treated as authoritative vault metadata.

The canonical information should remain in the vault's manifest/index.

For example, renaming:

```text
vault 60.300.pv
```

to:

```text
backup.pv
```

must not corrupt the logical vault.

---

# Segment Lifecycle

A vault may have an active segment while data is being written.

Example:

```text
vault 1.5.pv
vault 2.10.pv
vault 3.15.pv
active.pv
```

When the active segment reaches its configured size:

1. Stop writing to the current segment.
2. Finalize its metadata/integrity information.
3. Close the segment.
4. Give it its final filename.
5. Create the next segment.
6. Continue writing.

Closed segments should be treated as immutable whenever possible.

---

# Changing Segment Size

Changing the segment size changes the **physical representation** of the vault, not the logical contents.

For example:

```text
Current:

5 GiB × 70 segments
= 350 GiB
```

The user changes the setting to:

```text
10 GiB
```

PocketVault reorganizes the physical layout:

```text
Existing layout
      ↓
Repack
      ↓
10 GiB × ~35 segments
```

The logical contents and encryption semantics remain unchanged.

---

# Repacking

Changing segment size requires a **repack/reorganization operation**.

This operation may involve significant disk I/O.

For example, reorganizing a 2 TiB vault may require approximately:

```text
~2 TiB read
+
~2 TiB write
```

The exact duration depends on:

* HDD/SSD performance
* filesystem
* available disk space
* storage connection speed
* system I/O load

Repacking should therefore be treated as a potentially long-running operation.

---

# Segment Processing State

While a vault is being repacked, PocketVault must enter a dedicated processing state.

```text
IDLE
  ↓
PROCESSING_SEGMENT
  ↓
VERIFYING
  ↓
COMMITTING
  ↓
IDLE
```

If an error occurs:

```text
PROCESSING_SEGMENT
        ↓
       ERROR
        ↓
RECOVER / ROLLBACK
```

---

# Blocking UI During Processing

While segment processing is active, PocketVault must block interactions that could interfere with the vault.

The UI should display a blocking screen such as:

> **Segment is processing**
>
> PocketVault is reorganizing your vault.
>
> **Please do not close the application or disconnect the storage device.**

A progress indicator should be displayed where possible:

```text
Segment is processing

Reorganizing your vault...

██████████████░░░░░░ 68%

Please do not close the application
or disconnect the storage device.
```

During this state, the following operations should be disabled:

* Add files
* Delete files
* Move files
* Rename files
* Import/export
* Change password
* Change encryption settings
* Change segment size
* Open another vault
* Close the vault
* Normal application shutdown

The storage layer should enforce the lock as well; this must not rely solely on the UI.

---

# Repack Safety

Repacking must not destroy the original vault before the new layout has been successfully created and verified.

Recommended workflow:

```text
Existing vault
      ↓
Create temporary new layout
      ↓
Write/repack segments
      ↓
Verify new layout
      ↓
Commit new layout
      ↓
Remove old layout
```

The old layout should remain available until the new layout has been successfully verified.

This protects against:

* Application crashes
* Power loss
* Storage disconnection
* Insufficient disk space
* Unexpected I/O errors

An interrupted repack must leave the original vault recoverable.

---

# Temporary Storage Requirements

Repacking may temporarily require additional disk space.

For example:

```text
Existing layout
500 GiB

Temporary new layout
up to ~500 GiB
```

PocketVault should check available storage before starting a repack.

If sufficient space is unavailable, the operation should be rejected before modifying the existing vault.

---

# Format Versioning

Segmentation itself does not necessarily require changing the encryption format of existing chunks.

If the existing `.pv` structure already supports encrypted chunks as independent units, segments can act as physical containers around those chunks.

However, the `.pv` format should have an explicit **format version**.

For example:

```text
Magic:   PV
Version: 2
```

This allows PocketVault to distinguish between storage formats as the project evolves.

A format version upgrade should not automatically imply decrypting and re-encrypting existing data.

Where compatible, migration should follow:

```text
Existing encrypted chunks
        ↓
Read
        ↓
Pack
        ↓
New .pv segments
```

rather than:

```text
Decrypt
   ↓
Encrypt again
```

---

# Backward Compatibility

Existing PocketVault vaults should remain readable.

If an older vault uses the previous chunk-based layout:

```text
Old Vault
├── chunk
├── chunk
├── chunk
└── ...
```

PocketVault should be able to migrate it into the new segmented layout:

```text
Old encrypted chunks
        ↓
Pack
        ↓
vault 1.5.pv
vault 2.10.pv
...
```

Where possible, ciphertext should be preserved without re-encryption.

The migration process should be treated as a separate, explicit storage migration operation.

---

# Single-File Mode

Single-file storage can be supported as a special case where the entire vault is represented as one segment.

Example:

```text
MyVault/
└── vault.pv
```

Conceptually:

```text
segment count = 1
```

This allows users who prefer a single large file to use the same underlying storage model without requiring a completely separate format.

---

# Integrity

Segmentation is **not a security mechanism**.

It does not replace:

* AEAD authentication
* Chunk integrity
* Manifest validation
* Cryptographic verification
* Backup strategies

The segment filename provides only a convenient human-readable sanity check.

Cryptographic metadata remains the source of truth for determining whether encrypted data is valid.

---

# Non-Goals

Vault Segmentation is not intended to:

* Increase encryption performance
* Compress data
* Provide redundancy
* Replace backups
* Implement RAID
* Implement cloud synchronization
* Create a distributed filesystem
* Guarantee migration integrity by filename alone

Its purpose is to provide a **flexible physical representation of a large encrypted vault**.

---

# Core Product Principle

> **One logical vault, flexible physical storage.**

PocketVault should treat the vault as one logical object while allowing its physical representation to adapt to the user's needs.

A user may choose:

```text
1 GiB segments
```

when they want smaller migration units,

```text
5 GiB segments
```

as the balanced default,

```text
10–20 GiB segments
```

when they want fewer files,

or:

```text
Single File
```

when they prefer one large archive.

The underlying encrypted data remains the same; only its physical organization changes.

### Primary intention

> **Segmentation is an optimization for humans managing large amounts of physical data, not merely an optimization for computers.**

It makes large encrypted archives easier to **count, inspect, migrate, reorganize, and maintain over their lifetime**.
