# McBean Editor

McBean is a web-hosted service for viewing, editing, and proposing changes to [Tracey](https://tracey.bearcove.eu)-formatted specification files hosted in GitHub repositories.
It targets non-technical stakeholders who need to read and contribute to specs without interacting with Git, Markdown, or rule syntax directly.

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

r[view.modes.overview]
McBean MUST provide two view modes: a Reader view and a Developer view.

r[view.modes.reader]
Reader view is the default.
Raw Markdown syntax and rule ID markers MUST NOT be visible in Reader view.

r[view.modes.developer]
Developer view exposes rule ID badges alongside each rule, with affordances for copying IDs and viewing code reference counts.

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
The query interface in Reader view MUST be strictly read-only.
It MUST NOT be possible to initiate or modify a proposal from within the query interface.

r[view.query.preview]
McBean MUST allow the query interface to optionally include the content of open proposals in its context, so users can ask questions about the spec as it would look if a given proposal were merged.

## Rule IDs

r[ids.derived]
Rule IDs MUST be automatically derived at the time a rule is created, from the heading hierarchy and a slug of the rule's initial text.
Users MUST NOT be required to supply or understand rule IDs during normal editing.

r[ids.frozen]
Once assigned, a rule ID MUST NOT change automatically due to subsequent edits to the rule's text or position.
IDs are frozen at creation.

r[ids.stable-on-reorder]
Reordering rules or sections MUST NOT alter any rule IDs.
Rule IDs are not positional.

r[ids.collision]
When a derived slug would collide with an existing sibling ID, McBean MUST resolve the collision automatically by appending a numeric suffix.

r[ids.override]
Users in Developer view MUST be able to manually set the slug component of a new rule's ID before the rule is confirmed.
Once a rule is confirmed, its ID is frozen.

r[ids.rename]
Renaming a section MUST NOT automatically rename the IDs of rules within it.
If a user explicitly requests an ID rename, McBean MUST display the number of code references affected before the rename is applied.

r[ids.version-bump]
When rule text is modified, McBean MUST automatically increment the version suffix of that rule's ID in the generated Markdown, conforming to Tracey's versioning convention.

## WYSIWYG Editor

r[edit.availability]
The editor MUST only be available in the context of an active proposal.
A user MUST NOT be able to make changes to the spec outside of a proposal.

r[edit.rule-text]
Users MUST be able to edit the prose of any existing rule through a direct, inline editing interaction without any Markdown syntax being exposed.

r[edit.add-rule]
Users MUST be able to add a new rule within any section. The new rule MUST be assigned an ID automatically per r[ids.derived].

r[edit.add-section]
Users MUST be able to add new sections and nested subsections.
Section hierarchy MUST be visually represented and editable.

r[edit.reorder]
Users MUST be able to reorder both rules and sections via drag-and-drop.
Reordering MUST NOT affect any rule IDs per r[ids.stable-on-reorder].

r[edit.delete]
Users MUST be able to delete rules and sections.
Deleting a rule or section that has known code references MUST surface a warning before the deletion is confirmed.

r[edit.links.internal]
The editor MUST provide autocomplete assistance for linking to other rules within the same spec.
Typing a trigger character MUST present a searchable list of existing rule IDs and their prose summaries.

r[edit.links.external]
The editor MUST provide assistance for inserting external URLs, including link text suggestion.

r[edit.assist.prompt]
While editing within a proposal, users MUST be able to invoke an LLM-assisted edit via a prompt input.
The prompt MUST default in scope to the section currently in focus, with an option to widen scope to the full spec.

r[edit.assist.staging]
LLM-suggested edits MUST be presented in a staging area that is separate from the proposal itself.
Staged changes MUST NOT be applied to the proposal until the user explicitly accepts them, either in full or selectively per change.

r[edit.assist.no-structural-ops]
The LLM edit-assist MUST NOT be able to trigger proposal submission or perform any action outside of drafting content changes.

## Proposals

A proposal is a named, persistent draft of changes to a spec.

r[proposal.git.backing]
Each proposal corresponds to a dedicated branch in the backing repository.

r[proposal.git.exposure]
Users MUST NOT be exposed to branch names or Git concepts directly.

r[proposal.create.prompt]
When a user initiates a new proposal, McBean MUST present a prompt modal offering to generate an initial draft from a natural language description.
Submitting the prompt MUST simultaneously derive a candidate proposal title and populate the staging area with suggested content changes per r[edit.assist.staging].

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
When a user attempts to create a new proposal while one proposal is already open, McBean MUST display a warning indicating that an open proposal exists, before proceeding.

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
