- Updated the installer to detect signed release inventories and atomically
  plant authenticated `Distro.toml`, `Distro.lock`, and `Distro.sig` members
  with immutable `0600` permissions and matching BLAKE3 records.
