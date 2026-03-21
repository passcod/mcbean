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
    pub content: String,
}

#[server]
pub async fn get_spec(spec_id: i32) -> Result<SpecDetail, ServerFnError> {
    use diesel::prelude::*;

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

            Ok::<_, diesel::result::Error>(SpecDetail {
                id: sid,
                name: sname,
                repository_id: repo_id,
                files: files
                    .into_iter()
                    .map(|(fid, fpath, fcontent)| SpecFileDetail {
                        id: fid,
                        path: fpath,
                        content: fcontent,
                    })
                    .collect(),
            })
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact error: {e}")))?
        .map_err(|e| {
            tracing::error!(spec_id, error = %e, "get_spec query failed");
            ServerFnError::new(format!("query error: {e}"))
        })?;

    tracing::info!(
        spec_id,
        file_count = result.files.len(),
        "get_spec returned"
    );
    Ok(result)
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
                        view! {
                            <h1 class="title">{name}</h1>

                            {detail.files.into_iter().map(|file| {
                                let path = file.path.clone();
                                let lines: Vec<String> = file.content.lines().map(String::from).collect();
                                view! {
                                    <div class="box">
                                        <h2 class="subtitle is-5">
                                            <span class="icon-text">
                                                <span class="icon"><i class="fas fa-file-alt"></i></span>
                                                <span>{path}</span>
                                            </span>
                                        </h2>
                                        // r[impl view.render]
                                        <div class="content">
                                            <pre style="white-space: pre-wrap;">{
                                                lines.into_iter().map(|line| {
                                                    let trimmed = line.trim();
                                                    if trimmed.starts_with("r[") && trimmed.ends_with(']') {
                                                        let rule_id = trimmed[2..trimmed.len()-1].to_string();
                                                        view! {
                                                            <span
                                                                id=rule_id.clone()
                                                                class="has-text-grey-light is-size-7"
                                                            >
                                                                {line.clone()}
                                                            </span>
                                                            "\n"
                                                        }.into_any()
                                                    } else if trimmed.starts_with('#') {
                                                        view! {
                                                            <strong>{line.clone()}</strong>
                                                            "\n"
                                                        }.into_any()
                                                    } else {
                                                        view! {
                                                            <span>{line}</span>
                                                            "\n"
                                                        }.into_any()
                                                    }
                                                }).collect::<Vec<_>>()
                                            }</pre>
                                        </div>
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
