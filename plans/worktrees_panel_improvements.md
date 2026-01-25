# Worktrees Panel Improvements Plan

## Overview
Improve the worktrees panel to match project panel styling, add a + button for creating constellation-named worktrees, and enable agent persistence across worktree switches.

---

## Part 1: Style Matching

**Status**: The worktrees panel already uses the same core styling patterns as the project panel (ListItem, border_1/border_r_2, same color tokens). Minor enhancements needed.

**File**: `crates/worktrees_panel/src/worktrees_panel.rs`

### Changes:
1. Add a git branch icon (`IconName::GitBranch`) before each branch name in the `start_slot` of the ListItem
2. Update the header to include an icon and match project panel header styling

---

## Part 2: Add + Button with Constellation Names

**Files**:
- `crates/worktrees_panel/src/worktrees_panel.rs`
- `crates/worktrees_panel/Cargo.toml`

### Constellation Names (88 total):
```
andromeda, antlia, apus, aquarius, aquila, ara, aries, auriga,
bootes, caelum, camelopardalis, cancer, canes-venatici, canis-major,
canis-minor, capricornus, carina, cassiopeia, centaurus, cepheus,
cetus, chamaeleon, circinus, columba, coma-berenices, corona-australis,
corona-borealis, corvus, crater, crux, cygnus, delphinus, dorado,
draco, equuleus, eridanus, fornax, gemini, grus, hercules, horologium,
hydra, hydrus, indus, lacerta, leo, leo-minor, lepus, libra, lupus,
lynx, lyra, mensa, microscopium, monoceros, musca, norma, octans,
ophiuchus, orion, pavo, pegasus, perseus, phoenix, pictor, pisces,
piscis-austrinus, puppis, pyxis, reticulum, sagitta, sagittarius,
scorpius, sculptor, scutum, serpens, sextans, taurus, telescopium,
triangulum, triangulum-australe, tucana, ursa-major, ursa-minor, vela,
virgo, volans, vulpecula
```

### Implementation:
1. Add `rand` dependency to `Cargo.toml`
2. Add `CreateWorktree` action
3. Add `IconButton::new("add-worktree", IconName::Plus)` to the header
4. Implement `create_worktree()` method that:
   - Generates a random constellation name using `rand::seq::SliceRandom`
   - Gets the repo root path from the worktree registry
   - Creates worktree at sibling directory: `repo_root.parent().join(constellation_name)`
   - Uses existing `Repository::create_worktree()` logic from `crates/git/src/repository.rs:1408-1445`
   - Opens the new worktree (replaces current window)

---

## Part 3: Worktree Location

**Recommendation**: Sibling directory (git convention)

If repo is at `/Users/josh/Code/zed`, new worktree goes to `/Users/josh/Code/andromeda`

This is the standard git worktree convention and matches the existing behavior in `worktree_picker.rs:304`:
```rust
let new_worktree_path = path.join(branch);
```

For the + button, we skip the directory picker dialog and use the repo parent directly.

---

## Part 4: Agent Persistence (Phased Approach)

This is the most complex requirement. Current behavior: switching worktrees calls `replace_workspace_root()` which completely replaces the workspace, killing all agents.

### Phase 1 (This PR - Quick Win): "Open in New Window" Option

Provide two behaviors while keeping existing UX as default:
1. **Click**: Replace current window (existing behavior)
2. **Cmd+Click**: Open in new window (keeps current workspace/agents running)

**Changes to `crates/worktrees_panel/src/worktrees_panel.rs`**:
- Modify `switch_to_slot()` to check for Cmd modifier
- Without Cmd: call existing `replace_workspace_root()`
- With Cmd held: call new `open_worktree_in_new_window()` method

This gives users explicit control over whether to preserve their current agents while keeping the familiar default behavior.

### Phase 2 (Future): Background Agent Indicator
- Show visual indicator on worktree entries that have running agents in cached windows
- Extend cache timeout for worktrees with active agents (in `worktree_registry.rs`)

### Phase 3 (Future): Shared Agent Execution Service
- Extract agent execution to a global service that survives workspace changes
- Major architectural refactor - out of scope for this PR

---

## Files to Modify

| File | Changes |
|------|---------|
| `crates/worktrees_panel/src/worktrees_panel.rs` | Add + button, branch icons, open-in-new-window logic |
| `crates/worktrees_panel/Cargo.toml` | Add `rand` dependency |

---

## Verification

1. **Styling**: Open worktrees panel, verify entries have branch icons and match project panel visually
2. **+ Button**: Click +, verify new worktree created with constellation name at sibling path
3. **Agent Persistence**:
   - Start an agent conversation in worktree A
   - Click worktree B (should replace current window - existing behavior)
   - Cmd+click worktree C (should open new window, keeping A's agents running)
