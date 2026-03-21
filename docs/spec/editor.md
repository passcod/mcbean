# McBean Editor

McBean is a web-hosted service for viewing, editing, and proposing changes to [Tracey](https://tracey.bearcove.eu)-formatted specification files hosted in GitHub repositories.
It targets non-technical stakeholders who need to read and contribute to specs without interacting with Git, Markdown, or rule syntax directly.

## Users

r[users.identity]
McBean MUST be able to differentiate between distinct human users.
How authentication is performed is not specified.

r[users.collaboration]
All users MUST be able to view and contribute edits to any proposal that is in the Drafting state.
Proposals are not owned exclusively by their creator.

## Repository Connection

r[repo.connect]
An admin MUST be able to connect McBean to a GitHub repository by providing its URL.
McBean MUST verify that the repository contains at least one valid Tracey spec file before completing the connection.

r[repo.multi-spec]
McBean MUST support repositories that contain multiple Tracey specs.
Each spec MUST be navigable independently within the same repository context.

r[repo.multi-file]
McBean MUST support specs that span multiple files.
It is not required to support creating or renaming files.

## Spec Viewer

r[view.render]
McBean MUST render Tracey spec files as structured, human-readable documents.
Rule IDs MUST be visible alongside each rule, rendered in a visually insignificant style that does not distract from the prose content.

r[view.nav]
McBean MUST provide navigation within a spec, allowing users to jump to sections and rules directly.
Navigation MUST be available without scrolling through the full document.

r[view.search]
McBean MUST provide a search bar, which searches the spec using word matching.

r[view.query.chat]
McBean MUST provide a query interface that allows users to ask natural language questions about the spec.
The query interface MUST use the full spec content as context when generating answers.

r[view.query.references]
Answers from the query interface MUST include references to the specific rules that support each answer.
Activating a rule reference MUST scroll the spec to that rule and highlight it.

r[view.query.readonly]
The query interface in the default view MUST be strictly read-only.
It MUST NOT be possible to initiate or modify a proposal from within the query interface.

r[view.query.preview]
McBean MUST allow the query interface to optionally include the content of open proposals in its context, so users can ask questions about the spec as it would look if a given proposal were merged.

## Rule IDs

r[ids.provisional]
When a new rule is added during editing, it MUST immediately be assigned a provisional ID.
A provisional ID consists of a randomly generated hex string placed under the relevant heading hierarchy, with a `+0` version suffix (e.g. `r[security.3f8a2c1b+0]`).
Provisional IDs are placeholders only and are not considered stable.

r[ids.user-supplied]
A user MAY explicitly supply a slug for a new rule's ID at any point before the proposal is finalised.
A user-supplied slug MUST be used as-is during finalisation and MUST NOT be replaced by the LLM.

r[ids.finalise-phase]
When a proposal is marked as ready for review, McBean MUST run a finalisation phase before the proposal is submitted.
During finalisation, the full spec edit content — including all surrounding context — MUST be passed to an LLM for ID assignment.
The LLM MUST NOT modify any spec prose or structure during this phase.

r[ids.finalise-new]
During finalisation, every rule that still carries a provisional ID MUST be assigned a final ID by the LLM.
Final IDs MUST be derived from the rule's content and its position in the heading hierarchy, producing a concise, meaningful slug.

r[ids.finalise-existing]
During finalisation, every existing rule whose text was modified MUST be evaluated by the LLM to determine whether its current ID slug remains appropriate given the new content.
If the LLM determines the slug no longer fits, it MUST propose a replacement slug.

r[ids.finalise-approval]
After finalisation, McBean MUST present the full set of proposed ID assignments and changes to the user for review before the proposal is submitted.
The user MUST be able to approve, reject, or override any individual ID change.
Feedback provided during this review MUST be passed back to the LLM for a revised pass if any changes are rejected.

r[ids.no-collisions]
Rule ID slugs MUST be unique within a spec.
Collisions MUST NOT be resolved by appending numeric suffixes.
If a collision is detected during finalisation, the LLM MUST be prompted to produce a distinct slug that meaningfully differentiates the rules by content.

r[ids.stable-on-reorder]
Reordering rules or sections MUST NOT alter any finalised rule IDs.
Rule IDs are not positional.

r[ids.version-bump]
When rule text is modified, McBean MUST automatically increment the version suffix of that rule's ID in the generated Markdown, conforming to Tracey's versioning convention.

## WYSIWYG Editor

r[edit.availability]
The editor MUST only be available in the context of an active proposal.
A user MUST NOT be able to make changes to the spec outside of a proposal.

r[edit.rule-text]
Users MUST be able to edit the prose of any existing rule through a direct, inline editing interaction without any Markdown syntax being exposed.

r[edit.add-rule]
Users MUST be able to add a new rule within any section.
The new rule MUST immediately be assigned a provisional ID per r[ids.provisional].

r[edit.add-section]
Users MUST be able to add new sections and nested subsections.
Section hierarchy MUST be visually represented and editable.

r[edit.reorder]
Users MUST be able to reorder both rules and sections via drag-and-drop.
Reordering MUST NOT affect any finalised rule IDs per r[ids.stable-on-reorder].

r[edit.delete]
Users MUST be able to delete rules and sections.

r[edit.links.internal]
The editor MUST provide autocomplete assistance for linking to other rules within the same spec.
Typing a trigger character MUST present a searchable list of existing rule IDs and their prose summaries.

r[edit.links.external]
The editor MUST provide assistance for inserting external URLs, including link text suggestion.

r[edit.assist.prompt]
While editing within a proposal, users MUST be able to invoke an LLM-assisted edit via a prompt input.
The prompt MUST default in scope to the section currently in focus, with an option to widen scope to the full spec.

r[edit.assist.apply]
LLM-suggested edits MUST be applied directly to the proposal content.

r[edit.assist.no-structural-ops]
The LLM edit-assist MUST NOT be able to trigger proposal submission or perform any action outside of drafting content changes.

r[edit.undo]
McBean MUST provide unlimited undo for all changes made within a proposal, whether made by the user or by the LLM edit-assist.
Undo MUST be available at all times while the proposal is in the Drafting state.
Each LLM-assisted edit MUST be recorded as a discrete undoable step, not as a series of individual character changes.

r[edit.history]
McBean MUST retain the full change history of a proposal indefinitely, including after the proposal is merged.
The history MUST remain browsable at any point in the future.
Each history entry MUST record which user made the change.
For changes made by the LLM edit-assist, the entry MUST record the user on whose behalf the LLM acted, and the prompt that produced the change.

## Proposals

A proposal is a named, persistent draft of changes to a spec.

r[proposal.git.backing]
Each proposal corresponds to a dedicated branch in the backing repository.

r[proposal.git.exposure]
Users MUST NOT be exposed to branch names or Git concepts directly.

r[proposal.create.prompt]
When a user initiates a new proposal, McBean MUST present a prompt modal offering to generate an initial draft from a natural language description.
Submitting the prompt MUST simultaneously derive a candidate proposal title and apply suggested content changes to the proposal per r[edit.assist.apply].

r[proposal.create.dismiss]
If the user dismisses the prompt modal without submitting, an untitled proposal MUST be created and the editor opened immediately with no further interruption.

r[proposal.title.autogen]
When a proposal has no user-supplied title and contains at least one change, McBean MUST automatically derive a candidate title by passing the current diff to an LLM after a period of editing inactivity.
This MUST happen silently, without interrupting the user.

r[proposal.title.user-priority]
If a user has set a proposal title manually at any point, automatic title derivation MUST NOT overwrite it.

r[proposal.title.editable]
A proposal title MUST always be editable inline by the user, regardless of how it was set.

r[proposal.multiple.overview]
A user MAY have multiple open proposals simultaneously against the same repository.

r[proposal.multiple.warning]
When a user attempts to create a new proposal while they are already contributing to one or more open proposals, McBean MUST display a warning indicating this before proceeding.

r[proposal.diff.semantic]
McBean MUST present proposal changes as a semantic changelog derived from the parsed rule tree, not as a raw text diff.
The changelog MUST describe changes in plain language (e.g. "Modified rule", "Added section", "Reordered").

r[proposal.diff.expandable]
Each entry in the semantic changelog MUST be expandable to show a side-by-side comparison of the old and new rule prose, with some highlighting of the differences.

r[proposal.diff.version-bumps]
Version bump changes MUST be presented as a distinct, lower-prominence category in the changelog, separate from content additions and modifications.

r[proposal.submit]
Users MUST be able to submit a proposal for review.
Submission MUST create a pull request in the backing repository.
Users MUST NOT need to interact with GitHub to submit.

## Proposal Lifecycle

r[lifecycle.drafting]
A proposal that has no open implementation pull requests is in the Drafting state.
In this state the proposal is fully editable.

r[lifecycle.in-progress.trigger]
When one or more implementation pull requests targeting the proposal's branch are opened in the backing repository, the proposal MUST automatically transition to the In Progress state.

r[lifecycle.in-progress.frozen]
While a proposal is In Progress, the spec diff for that proposal MUST be frozen.
Editing the proposal content MUST NOT be possible until all implementation pull requests have been resolved.

r[lifecycle.in-progress.amendment]
GitHub committers may edit the spec proposal in the backing branch directly (or via their own PRs).
McBean SHOULD keep track of this and show amendments to the spec in the frozen proposal interface.

r[lifecycle.abandoned]
If the backing branch's PR is closed without merging, the proposal MUST return to the Drafting state.
McBean MUST indicate that implementation was previously abandoned.

r[lifecycle.merged]
When the proposal branch is merged into the repository's main branch, the proposal transitions to the Merged state.

## Notifications

r[notify.slack]
McBean MUST support sending a notification to a configured Slack channel when a proposal is submitted for review.
The Slack webhook URL MUST be configurable per repository.