use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::{components::*, path};

use crate::components::Nav;
use crate::pages::{HomePage, NewProposalPage, ProposalPage, RepoPage};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <link
                    rel="stylesheet"
                    href="https://cdn.jsdelivr.net/npm/bulma@1.0.2/css/bulma.min.css"
                />
                <AutoReload options=options.clone() />
                <HydrationScripts options />
                <MetaTags />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

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
