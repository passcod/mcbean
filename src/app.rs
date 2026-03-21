use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::{components::*, path};

use crate::components::Nav;
use crate::pages::{HomePage, NewProposalPage, ProposalPage, RepoPage};

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Nav />
        <Router>
            <main class="section">
                <div class="container">
                    <Routes fallback=|| "Not found">
                        <Route path=path!("/") view=HomePage />
                        <Route path=path!("/repo/:repo_id") view=RepoPage />
                        <Route path=path!("/repo/:repo_id/proposal/new") view=NewProposalPage />
                        <Route
                            path=path!("/repo/:repo_id/proposal/:proposal_id")
                            view=ProposalPage
                        />
                    </Routes>
                </div>
            </main>
        </Router>
    }
}
