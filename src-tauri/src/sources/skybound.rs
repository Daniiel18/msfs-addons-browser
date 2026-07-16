//! (v6.2.38) Fuente **Skybound** (https://skybound.cx/msfs-2024).
//!
//! Simplaza no publica addons de MSFS2024, así que en esa versión del
//! sim ofrecemos Skybound como catálogo alternativo (aviones/liveries/…;
//! NO escenarios, que ya cubre SceneryAddons). La página requiere INICIO
//! DE SESIÓN, así que mantenemos una sesión (cookies) tras autenticar con
//! las credenciales que el usuario guarda en Ajustes.
//!
//! ESTADO: SCAFFOLDING. La estructura, la sesión y el cableado están
//! listos; los SELECTORES de scraping y el endpoint/campos exactos del
//! LOGIN están marcados con `TODO(skybound)` — se completan con una
//! muestra del HTML real de la página (login + resultados).

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{Addon, BrowsePage, Source, SourceError};

const BASE: &str = "https://skybound.cx";
const CATALOG_PATH: &str = "/msfs-2024";

/// Credenciales + sesión compartidas. Las puebla el comando
/// `skybound_set_credentials` (y el bootstrap desde la DB); la fuente las
/// lee para autenticar de forma perezosa.
#[derive(Default)]
pub struct SkyboundAuth {
    pub username: Option<String>,
    pub password: Option<String>,
    /// Cookie de sesión tras el login (Set-Cookie). `None` = no logueado.
    pub session_cookie: Option<String>,
}

pub struct SkyboundSource {
    client: reqwest::Client,
    auth: Arc<RwLock<SkyboundAuth>>,
}

impl SkyboundSource {
    pub fn new(client: reqwest::Client, auth: Arc<RwLock<SkyboundAuth>>) -> Self {
        Self { client, auth }
    }

    /// ¿Hay credenciales configuradas?
    pub async fn has_credentials(auth: &Arc<RwLock<SkyboundAuth>>) -> bool {
        let a = auth.read().await;
        a.username.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
            && a.password.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
    }

    /// Devuelve una cookie de sesión válida, autenticando si hace falta.
    async fn ensure_session(&self) -> Result<String, SourceError> {
        {
            let a = self.auth.read().await;
            if let Some(c) = &a.session_cookie {
                if !c.is_empty() {
                    return Ok(c.clone());
                }
            }
        }
        // Sin sesión → login con las credenciales guardadas.
        let (user, pass) = {
            let a = self.auth.read().await;
            match (a.username.clone(), a.password.clone()) {
                (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => (u, p),
                _ => {
                    return Err(SourceError::Other(
                        "Skybound: configura tu usuario y contraseña en Ajustes".into(),
                    ))
                }
            }
        };
        let cookie = self.login(&user, &pass).await?;
        {
            let mut a = self.auth.write().await;
            a.session_cookie = Some(cookie.clone());
        }
        Ok(cookie)
    }

    /// Autentica contra Skybound y devuelve la cookie de sesión.
    ///
    /// TODO(skybound): endpoint y campos reales del formulario de login
    /// (se determinan del HTML de la página de login). Estructura típica
    /// WordPress/WooCommerce: POST a `/wp-login.php` con `log`, `pwd`,
    /// `rememberme`, `redirect_to` → cookies `wordpress_logged_in_*`.
    async fn login(&self, _user: &str, _pass: &str) -> Result<String, SourceError> {
        // Placeholder: cuando tengamos el HTML del login, hacemos el POST
        // y extraemos la(s) cookie(s) de `Set-Cookie`.
        tracing::warn!(
            target: "skybound",
            "login() aún no implementado (falta el HTML del formulario de login)"
        );
        Err(SourceError::Other(
            "Skybound: login todavía no configurado (scaffolding)".into(),
        ))
    }

    /// Descarga una URL con la cookie de sesión.
    #[allow(dead_code)]
    async fn fetch_authed(&self, url: &str) -> Result<String, SourceError> {
        let cookie = self.ensure_session().await?;
        let resp = self
            .client
            .get(url)
            .header(reqwest::header::COOKIE, cookie)
            .send()
            .await
            .map_err(SourceError::Http)?;
        resp.text().await.map_err(SourceError::Http)
    }

    /// Parsea el HTML de una página de resultados de Skybound a `Addon`s.
    ///
    /// TODO(skybound): selectores reales (tarjeta, título, dev, imagen,
    /// enlace de descarga). Mismo `Addon` que las otras fuentes.
    fn parse_results(&self, _html: &str) -> Vec<Addon> {
        // Placeholder hasta tener el HTML de resultados.
        Vec::new()
    }
}

#[async_trait]
impl Source for SkyboundSource {
    fn id(&self) -> &'static str {
        "skybound"
    }
    fn name(&self) -> &'static str {
        "Skybound"
    }
    fn home_url(&self) -> &'static str {
        BASE
    }

    async fn search(&self, query: &str) -> Result<Vec<Addon>, SourceError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        // TODO(skybound): URL de búsqueda real (¿`?s=` como WordPress?).
        let url = format!("{}{}/?s={}", BASE, CATALOG_PATH, urlencoding::encode(q));
        let html = self.fetch_authed(&url).await?;
        Ok(self.parse_results(&html))
    }

    async fn browse(&self, page: usize) -> Result<BrowsePage, SourceError> {
        // TODO(skybound): paginación real del catálogo /msfs-2024.
        let url = if page <= 1 {
            format!("{}{}/", BASE, CATALOG_PATH)
        } else {
            format!("{}{}/page/{}/", BASE, CATALOG_PATH, page)
        };
        let html = self.fetch_authed(&url).await?;
        let addons = self.parse_results(&html);
        let has_more = !addons.is_empty();
        Ok(BrowsePage {
            addons,
            page,
            has_more,
        })
    }
}
