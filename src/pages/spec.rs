use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpecDetail {
    pub id: i32,
    pub name: String,
    pub repository_id: i32,
    pub files: Vec<SpecFileDetail>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpecFileDetail {
    pub id: i32,
    pub path: String,
    /// Pre-rendered HTML from marq.
    pub html: String,
}

#[server]
pub async fn get_spec(spec_id: i32) -> Result<SpecDetail, ServerFnError> {
    use diesel::prelude::*;
    use marq::{RenderOptions, render};

    tracing::info!(spec_id, "get_spec called");

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool error: {e}")))?;

    let result = conn
        .interact(move |conn| {
            use crate::db::schema::{spec_files, specs};

            let (sid, sname, repo_id): (i32, String, i32) = specs::table
                .filter(specs::id.eq(spec_id))
                .select((specs::id, specs::name, specs::repository_id))
                .first(conn)?;

            // r[impl repo.multi-file]
            let files = spec_files::table
                .filter(spec_files::spec_id.eq(sid))
                .select((spec_files::id, spec_files::path, spec_files::content))
                .order(spec_files::path.asc())
                .load::<(i32, String, String)>(conn)?;

            Ok::<_, diesel::result::Error>((sid, sname, repo_id, files))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact error: {e}")))?
        .map_err(|e| {
            tracing::error!(spec_id, error = %e, "get_spec query failed");
            ServerFnError::new(format!("query error: {e}"))
        })?;

    let (sid, sname, repo_id, files) = result;

    let mut rendered_files = Vec::with_capacity(files.len());
    for (fid, fpath, fcontent) in files {
        let opts = RenderOptions::new().with_source_path(&fpath);
        // r[impl view.render]
        let doc = render(&fcontent, &opts).await.map_err(|e| {
            tracing::error!(spec_id, path = %fpath, error = %e, "marq render failed");
            ServerFnError::new(format!("Failed to render {fpath}: {e}"))
        })?;
        rendered_files.push(SpecFileDetail {
            id: fid,
            path: fpath,
            html: doc.html,
        });
    }

    tracing::info!(
        spec_id,
        file_count = rendered_files.len(),
        "get_spec returned"
    );

    Ok(SpecDetail {
        id: sid,
        name: sname,
        repository_id: repo_id,
        files: rendered_files,
    })
}

#[component]
pub fn SpecPage() -> impl IntoView {
    let params = use_params_map();
    let repo_id = move || {
        params
            .read()
            .get("repo_id")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0)
    };
    let spec_id = move || {
        params
            .read()
            .get("spec_id")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0)
    };

    let spec = Resource::new(spec_id, get_spec);

    view! {
        <nav class="breadcrumb" aria-label="breadcrumbs">
            <ul>
                <li><a href="/">"Repositories"</a></li>
                <li><a href=move || format!("/repo/{}", repo_id())>"Repository"</a></li>
                <li class="is-active"><a href="#">"Spec"</a></li>
            </ul>
        </nav>

        <Suspense fallback=move || view! { <p>"Loading spec..."</p> }>
            {move || {
                spec.get().map(|result| match result {
                    Ok(detail) => {
                        let name = detail.name.clone();
                        let propose_href =
                            format!("/repo/{}/proposal/new", repo_id());
                        view! {
                            <div class="level">
                                <div class="level-left">
                                    <div class="level-item">
                                        <h1 class="title">{name}</h1>
                                    </div>
                                </div>
                                <div class="level-right">
                                    <div class="level-item">
                                        <a class="button is-primary" href=propose_href>
                                            "Propose a Change"
                                        </a>
                                    </div>
                                </div>
                            </div>

                            {detail.files.into_iter().map(|file| {
                                let path = file.path.clone();
                                view! {
                                    <div class="box">
                                        <p class="has-text-grey is-size-7 mb-3">{path}</p>
                                        <div class="content spec-content" inner_html=file.html />
                                    </div>
                                }
                            }).collect::<Vec<_>>()}
                        }.into_any()
                    }
                    Err(e) => view! {
                        <div class="notification is-danger">
                            {format!("Error loading spec: {e}")}
                        </div>
                    }.into_any(),
                })
            }}
        </Suspense>
    }
}
