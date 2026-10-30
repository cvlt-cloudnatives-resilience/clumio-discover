# Changelog

## 0.3.2 - 2026-10-30

- Downloads have plainer names: `linux-x86_64`, `linux-aarch64`, `macos-arm64`, `macos-x86_64` and `windows-x86_64`.
- Release notes come from this changelog.
- The disclaimer is at the end of the README.

## 0.3.1 - 2026-10-29

- EBS shows two sizes: Provisioned, and Written, the full size of the volume's latest snapshot.
- S3 sizes always include the Intelligent-Tiering Archive Access and Deep Archive Access tiers and Reduced Redundancy storage. Per-object overhead from `--deep` is listed in the details but no longer added to the bucket size.
- A resource backed up only outside AWS Backup is a Medium finding. High is kept for resources with no backup of any kind.
- The README explains how sizes are measured, what each finding severity means, and the counts on the Overview.

## 0.3.0 - 2026-10-26

- Each resource is either AWS Backup or Not protected. Backups made outside AWS Backup are listed in the resource's details.
- The Overview bars use the full width of the screen.
- The README explains how protection is decided, and how to download and install.

## 0.2.0 - 2026-10-26

- First release: a read-only inventory of S3, EBS, EC2, RDS, DynamoDB, DocumentDB, Neptune, S3 Tables and Glue Iceberg in every region Clumio supports, with AWS Backup coverage, findings, a terminal browser, and JSON and CSV export.
