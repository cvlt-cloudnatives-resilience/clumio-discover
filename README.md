# Clumio Discover

See everything in your AWS account that Clumio can protect, and how well it's
protected today.

> Provided as-is by the Commvault Cloud Natives Resilience team. Not a supported
> Commvault product. Clumio and Commvault are trademarks of Commvault Systems,
> Inc.

![Clumio Discover overview](docs/overview.png)

- **Read-only.** Only Describe, List and Get calls. Nothing in your account is
  created, changed or deleted.
- **Nothing to install in AWS.** No CloudFormation, roles or agents. It runs on
  your machine with your existing AWS login.
- **Stays with you.** Results are shown on screen or saved to a local file.
  Nothing is sent anywhere.

## What you need

1. The `clumio-discover` program for your platform, from the
   [Releases](../../releases) page.
2. An AWS login for the account to scan, the same one you use with the AWS CLI.
3. Read access. The AWS managed policy **ReadOnlyAccess** (or admin) is enough.
   For least privilege, use the
   [`clumio-discover-readonly.json`](policy/clumio-discover-readonly.json)
   policy included in the download instead.

## Run it

```
clumio-discover
```

It scans every AWS region Clumio supports, saves the results to
`inventory-<account>-<date>.json` in the current folder, then opens the
inventory browser. To use a different AWS profile:

```
clumio-discover --profile my-profile
```

On macOS, if you're told the program can't be opened because it's from an
unidentified developer, clear the download flag once:

```
xattr -d com.apple.quarantine clumio-discover
```

On Windows, if SmartScreen warns about an unrecognised app, choose **More
info**, then **Run anyway**. The Linux builds run on any Linux distribution,
including Amazon Linux 2 and 2023.

## Find your way around

| Key | Action |
|---|---|
| `Tab` / `←` `→` | Switch section: Overview, S3, EBS, EC2, RDS, DynamoDB, Findings… |
| `↑` `↓` | Move through a list |
| `Enter` / `Esc` | Open / close the details of a resource |
| `/` | Search |
| `f` | Filter, for example only unprotected resources |
| `e` | Export the current list to CSV and JSON |
| `q` | Quit |

Each resource is marked:

- **AWS Backup**: backed up by AWS Backup.
- **Native only**: no AWS Backup, but it has snapshots, AMIs, automated backups
  or point-in-time recovery.
- **Unprotected**: no backup of any kind.

![Clumio Discover resource list](docs/resources.png)

**Findings** lists risks worth fixing, most serious first, such as unencrypted
volumes, tables without point-in-time recovery, or publicly shared snapshots.

## Deep scan

```
clumio-discover --deep
```

Also checks every EBS snapshot, RDS snapshot and AMI for public or
cross-account sharing. Recommended; on accounts with tens of thousands of
snapshots it takes a few minutes longer.

## Share the results

Every scan saves its results to `inventory-<account>-<date>.json`. Send that
file back to whoever asked you to run the scan, or open it again later on any
machine. Opening it needs no AWS access:

```
clumio-discover view inventory-123456789012-20261023T091400.json
```

The file contains resource names, IDs and tags from your account. Treat it as
confidential. To skip saving, add `--no-save`; to save without opening the
browser, use `--out <file>`.

## Options

Optional settings go in `~/.config/clumio-discover/config.toml`. Copy
[`config.example.toml`](config.example.toml) to get started; it covers
choosing regions, turning services off, and scanning only resources with a
given tag.

## Check that nothing changed

Every call the tool makes is recorded in AWS CloudTrail. In the CloudTrail
console, open **Event history**, filter by the user that ran the scan, and
confirm every event is read-only.

## Missing permissions

If your login can't read something, the scan still finishes. What it couldn't
read is listed in the **Errors** section, so you can add the permission and
scan again.

## License

[Apache License 2.0](LICENSE). Copyright 2026 Tirslo.
