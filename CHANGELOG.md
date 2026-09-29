# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.17.0] - 2026-09-29

### Added

- Workflow steps can declare `script:` instead of `prompt:` to run a shell
  script directly (no model, no cost); the script's stdout feeds later steps
  through `{{steps.<name>}}` placeholders, and a step defining both (or
  neither) fails validation before the run starts. (#50, 69b604d)
- Workflow runs now push their review fixes as pull requests, and each
  release is tagged on its version bump. (9ee5090)

## [0.16.1] - 2026-09-28

### Fixed

- The landing page now works on mobile. (4cfe89b)

### Changed

- Updated the required Node and pnpm versions. (5a0de8e, 9f9d41a)
