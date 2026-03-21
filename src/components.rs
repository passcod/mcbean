pub mod changelog_sidebar;
pub mod editor;
pub mod finalise_fab;
pub mod loro_doc;
pub mod nav;
pub mod proposal_fab;
pub mod sidebar;
pub mod spec_block_editor;

pub use changelog_sidebar::{ChangelogSidebar, compute_changelog};
pub use editor::Editor;
pub use finalise_fab::FinaliseFab;
pub use nav::Nav;
pub use proposal_fab::ProposalFab;
pub use sidebar::{HeadingEntry, SearchEntry, SpecOutline, SpecSidebar};
pub use spec_block_editor::{
    RevertOp, SpecBlock, SpecBlockEditor, SpecBlockKind, blocks_to_sidebar_data, get_proposal_doc,
    parse_blocks_from_content, sync_proposal,
};
