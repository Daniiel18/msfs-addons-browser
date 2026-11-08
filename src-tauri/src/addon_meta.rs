//! (06/11) Tipo y etiquetas que el usuario da a sus addons, para editarlos en
//! lote desde Instalados. Se guarda en SimFleet: nunca toca archivos del addon.

use anyhow::{bail, Result};
use sqlx::SqlitePool;

/// Lo que `derivedType` entiende y puede salir en una categoría de Instalados.
const USER_TYPES: [&str; 4] = ["AIRCRAFT", "LIVERY", "INSTRUMENT", "MISC"];
const MAX_TAG: usize = 32;

/// Tipo de unos addons. `None` vuelve al que detecta SimFleet.
pub async fn set_type(pool: &SqlitePool, folders: &[String], user_type: Option<&str>) -> Result<()> {
    if user_type.is_some_and(|t| !USER_TYPES.contains(&t)) {
        bail!("unknown_type");
    }
    let mut tx = pool.begin().await?;
    for folder in folders {
        sqlx::query(
            r#"INSERT INTO addon_user_meta (folder_name, user_type) VALUES (?1, ?2)
               ON CONFLICT(folder_name) DO UPDATE SET
                   user_type = excluded.user_type,
                   updated_at = datetime('now')"#,
        )
        .bind(folder)
        .bind(user_type)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Sin espacios de más, recortada, y con una sola grafía por etiqueta.
fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag: String = tag.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_TAG).collect();
        if !tag.is_empty() && !out.iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
            out.push(tag);
        }
    }
    out
}

fn merged(current: Vec<String>, add: &[String], remove: &[String]) -> Vec<String> {
    let gone = |t: &String| remove.iter().any(|r| r.to_lowercase() == t.to_lowercase());
    let mut out: Vec<String> = current.into_iter().filter(|t| !gone(t)).collect();
    out.extend(add.iter().cloned());
    clean_tags(&out)
}

/// Pone y quita etiquetas en unos addons; las demás se quedan.
pub async fn edit_tags(pool: &SqlitePool, folders: &[String], add: &[String], remove: &[String]) -> Result<()> {
    let add = clean_tags(add);
    let remove = clean_tags(remove);
    let mut tx = pool.begin().await?;
    for folder in folders {
        let saved: Option<(String,)> = sqlx::query_as("SELECT tags FROM addon_user_meta WHERE folder_name = ?1")
            .bind(folder)
            .fetch_optional(&mut *tx)
            .await?;
        let current = saved.and_then(|(json,)| serde_json::from_str(&json).ok()).unwrap_or_default();
        sqlx::query(
            r#"INSERT INTO addon_user_meta (folder_name, tags) VALUES (?1, ?2)
               ON CONFLICT(folder_name) DO UPDATE SET
                   tags = excluded.tags,
                   updated_at = datetime('now')"#,
        )
        .bind(folder)
        .bind(serde_json::to_string(&merged(current, &add, &remove))?)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn list(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    async fn meta(pool: &SqlitePool, folder: &str) -> (Option<String>, Vec<String>) {
        let (user_type, tags): (Option<String>, String) =
            sqlx::query_as("SELECT user_type, tags FROM addon_user_meta WHERE folder_name = ?1")
                .bind(folder)
                .fetch_one(pool)
                .await
                .unwrap();
        (user_type, serde_json::from_str(&tags).unwrap())
    }

    #[test]
    fn tags_are_tidied_up() {
        assert_eq!(clean_tags(&list(&["  Largo   radio ", "largo radio", "", "Corto"])), ["Largo radio", "Corto"]);
        assert_eq!(clean_tags(&list(&[&"x".repeat(50)]))[0].len(), MAX_TAG);
    }

    #[tokio::test]
    async fn type_and_tags_do_not_step_on_each_other() {
        let pool = db().await;
        let folders = list(&["a", "b"]);

        set_type(&pool, &folders, Some("LIVERY")).await.unwrap();
        edit_tags(&pool, &folders, &list(&["Brasil", "Largo"]), &[]).await.unwrap();
        edit_tags(&pool, &list(&["b"]), &list(&["Corto"]), &list(&["brasil"])).await.unwrap();

        assert_eq!(meta(&pool, "a").await, (Some("LIVERY".to_string()), list(&["Brasil", "Largo"])));
        assert_eq!(meta(&pool, "b").await, (Some("LIVERY".to_string()), list(&["Largo", "Corto"])));

        set_type(&pool, &list(&["a"]), None).await.unwrap();
        assert_eq!(meta(&pool, "a").await, (None, list(&["Brasil", "Largo"])), "vuelve al automático; las etiquetas se quedan");
    }

    #[tokio::test]
    async fn an_unknown_type_is_refused() {
        let pool = db().await;
        assert!(set_type(&pool, &list(&["a"]), Some("SCENERY")).await.is_err());
    }

    #[tokio::test]
    async fn the_package_list_carries_them() {
        let pool = db().await;
        sqlx::query("INSERT INTO community_packages (folder_name, install_path, title) VALUES ('a', 'C:/a', 'A')")
            .execute(&pool)
            .await
            .unwrap();
        set_type(&pool, &list(&["A"]), Some("MISC")).await.unwrap();
        edit_tags(&pool, &list(&["a"]), &list(&["Brasil"]), &[]).await.unwrap();

        let rows = crate::db::repo::list_community_packages(&pool).await.unwrap();
        assert_eq!(rows[0].user_type.as_deref(), Some("MISC"));
        assert_eq!(rows[0].user_tags_json.as_deref(), Some(r#"["Brasil"]"#));
    }
}
