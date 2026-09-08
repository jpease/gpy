#!/usr/bin/env bash
# Generate a CycloneDX 1.5 SBOM for the gpy-agent workspace.
#
# Added in gpy#494. Every published release ships an SBOM alongside the
# binaries so downstream consumers can answer "is GPY affected by CVE-X"
# without cloning the repository and resolving Cargo.lock themselves.
#
# The document is derived from `cargo metadata --locked`, so it describes the
# dependency set the release binaries were actually built from -- a drifted
# Cargo.lock fails the run rather than producing a plausible-looking SBOM for
# a different graph.
#
# Scope: the transitive closure of the root package's normal and build
# dependencies. Dev-dependencies (criterion, tempfile, the test harness crates)
# are excluded because they are not linked into a shipped binary.
#
# Deliberately not using cargo-cyclonedx or syft: both would add a
# network-installed tool to the release pipeline that the local quality gate
# cannot run, so the SBOM step would only ever be exercised by a tag push --
# the exact failure mode #492 was filed for. cargo and jq are already gate
# dependencies.
#
# Usage:
#   scripts/generate-sbom.sh [--output sbom.cdx.json] [--version v0.1.0]
#
# --version stamps the release tag into metadata.component.version; without it
# the version comes from Cargo.toml.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$REPO_ROOT/gpy-agent/Cargo.toml"

OUTPUT=""
VERSION=""

die() {
    echo "generate-sbom: $*" >&2
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --output) OUTPUT="${2:-}"; shift 2 ;;
        --version) VERSION="${2:-}"; shift 2 ;;
        -h|--help) sed -n '2,28p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 2 ;;
        *) die "unknown argument: $1" ;;
    esac
done

command -v cargo >/dev/null 2>&1 || die "cargo not found"
command -v jq >/dev/null 2>&1 || die "jq not found"
[[ -f "$MANIFEST" ]] || die "manifest not found: $MANIFEST"

if [[ -n "$VERSION" ]]; then
    [[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "invalid version: $VERSION (expected vMAJOR.MINOR.PATCH)"
    VERSION="${VERSION#v}"
fi

# SOURCE_DATE_EPOCH support keeps the document byte-identical across two runs
# of the same commit, so a rebuild can be diffed against the published SBOM.
if [[ -n "${SOURCE_DATE_EPOCH:-}" ]]; then
    TIMESTAMP="$(date -u -r "$SOURCE_DATE_EPOCH" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
        date -u -d "@$SOURCE_DATE_EPOCH" +%Y-%m-%dT%H:%M:%SZ)"
else
    TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
fi

metadata="$(cargo metadata --locked --format-version 1 --manifest-path "$MANIFEST")" ||
    die "cargo metadata failed (is Cargo.lock in sync with Cargo.toml?)"

sbom="$(jq -S \
    --arg timestamp "$TIMESTAMP" \
    --arg version_override "$VERSION" '
    # Walk the resolve graph from the root package, following only normal and
    # build edges. dep_kinds entries carry kind == null for a normal dependency,
    # "build" for a build-dependency and "dev" for a dev-dependency.
    def reachable($edges; $root):
        {seen: {}, frontier: [$root]}
        | until(.frontier | length == 0;
            . as $state
            | ($state.frontier | map(select($state.seen[.] | not)) | unique) as $new
            | {
                seen: ($state.seen + ($new | map({key: ., value: true}) | from_entries)),
                frontier: ($new | map($edges[.] // []) | add // [])
              })
        | .seen | keys;

    def component($pkg):
        {
            type: "library",
            "bom-ref": ("pkg:cargo/" + $pkg.name + "@" + $pkg.version),
            name: $pkg.name,
            version: $pkg.version,
            purl: ("pkg:cargo/" + $pkg.name + "@" + $pkg.version)
        }
        + (if $pkg.description then {description: $pkg.description} else {} end)
        + (if $pkg.license then {licenses: [{expression: $pkg.license}]} else {} end)
        + (if $pkg.repository then
              {externalReferences: [{type: "vcs", url: $pkg.repository}]}
           else {} end);

    .resolve.root as $root
    | ([.resolve.nodes[]
        | {
            key: .id,
            value: [.deps[]
                    | select(any(.dep_kinds[]; .kind == null or .kind == "build"))
                    | .pkg]
          }] | from_entries) as $edges
    | reachable($edges; $root) as $ids
    | ([.packages[] | {key: .id, value: .}] | from_entries) as $by_id
    | ($by_id[$root]) as $root_pkg
    | {
        bomFormat: "CycloneDX",
        specVersion: "1.5",
        version: 1,
        metadata: {
            timestamp: $timestamp,
            tools: [{vendor: "gpy", name: "scripts/generate-sbom.sh"}],
            component: (component($root_pkg)
                        | .type = "application"
                        | if $version_override != "" then
                              .version = $version_override
                              | ."bom-ref" = ("pkg:cargo/" + $root_pkg.name + "@" + $version_override)
                              | .purl = ("pkg:cargo/" + $root_pkg.name + "@" + $version_override)
                          else . end)
        },
        components: [$ids[]
                     | select(. != $root)
                     | component($by_id[.])]
        | sort_by(.name, .version)
      }
' <<<"$metadata")" || die "failed to build the CycloneDX document"

component_count="$(jq '.components | length' <<<"$sbom")"
[[ "$component_count" -gt 0 ]] || die "SBOM contains no components; the resolve graph was not walked correctly"

if [[ -n "$OUTPUT" ]]; then
    printf '%s\n' "$sbom" >"$OUTPUT"
    echo "generate-sbom: wrote $OUTPUT ($component_count components)"
else
    printf '%s\n' "$sbom"
fi
