using HtmlAgilityPack;
using SceneryAddonsBrowser.Logging;
using SceneryAddonsBrowser.Models;
using System.Net.Http;
using System.Text.RegularExpressions;

namespace SceneryAddonsBrowser.Services
{
    public class SearchService
    {
        private static readonly HttpClient _httpClient = new HttpClient
        {
            Timeout = TimeSpan.FromSeconds(15)
        };

        public async Task<List<Scenario>> SearchByIcaoAsync(string icao)
        {
            AppLogger.Log($"Searching ICAO: {icao}");

            var results = new List<Scenario>();

            if (string.IsNullOrWhiteSpace(icao))
                return results;

            icao = icao.Trim().ToUpperInvariant();

            string searchUrl = $"https://sceneryaddons.org/?s={icao}";
            string html;

            try
            {
                html = await _httpClient.GetStringAsync(searchUrl);
            }
            catch
            {
                return results;
            }

            var htmlDoc = new HtmlAgilityPack.HtmlDocument();
            htmlDoc.LoadHtml(html);

            var articles = htmlDoc.DocumentNode.SelectNodes("//article");
            if (articles == null)
                return results;

            foreach (var article in articles)
            {
                try
                {
                    var titleNode = article.SelectSingleNode(".//h2//a");
                    if (titleNode == null)
                        continue;

                    string title = HtmlEntity.DeEntitize(titleNode.InnerText.Trim());
                    string pageUrl = titleNode.GetAttributeValue("href", string.Empty);

                    // ================= ICAO EXACTO =================
                    string? extractedIcao = ExtractIcaoFromTitle(title);
                    if (extractedIcao == null)
                        continue;

                    if (!extractedIcao.Equals(icao, StringComparison.OrdinalIgnoreCase))
                        continue;

                    // ================= SOLO MSFS 2020 =================
                    bool isMsfs2020 =
                        title.Contains("MSFS 2020", StringComparison.OrdinalIgnoreCase) ||
                        title.Contains("MSFS 2020/2024", StringComparison.OrdinalIgnoreCase);

                    if (!isMsfs2020)
                        continue;

                    string simulator =
                        title.Contains("2024", StringComparison.OrdinalIgnoreCase)
                            ? "MSFS 2020/2024"
                            : "MSFS 2020";

                    var scenario = new Scenario
                    {
                        Icao = extractedIcao,
                        Name = title,
                        Developer = "Unknown",
                        Simulator = simulator,
                        Version = "Unknown",
                        SourcePageUrl = pageUrl
                    };

                    NormalizeNameAndDeveloper(scenario);

                    var downloadMethods = await GetDownloadMethodsAsync(pageUrl);
                    foreach (var method in downloadMethods)
                        method.Scenario = scenario;

                    scenario.DownloadMethods.AddRange(downloadMethods);
                    results.Add(scenario);
                }
                catch
                {
                    continue;
                }
            }

            AppLogger.Log($"Search complete. Returning {results.Count} scenarios.");
            return results;
        }

        // ================= DOWNLOAD METHODS =================
        private async Task<List<DownloadMethod>> GetDownloadMethodsAsync(string pageUrl)
        {
            var methods = new List<DownloadMethod>();
            if (string.IsNullOrWhiteSpace(pageUrl))
                return methods;

            string html;
            try
            {
                html = await _httpClient.GetStringAsync(pageUrl);
            }
            catch
            {
                return methods;
            }

            var doc = new HtmlAgilityPack.HtmlDocument();
            doc.LoadHtml(html);

            var links = doc.DocumentNode.SelectNodes("//a[contains(@href,'get.php')]");
            if (links == null)
                return methods;

            foreach (var link in links)
            {
                string href = link.GetAttributeValue("href", string.Empty);
                if (string.IsNullOrWhiteSpace(href))
                    continue;

                string text = HtmlEntity.DeEntitize(link.InnerText).Trim().ToLower();

                DownloadType type =
                    text.Contains("torrent")
                        ? DownloadType.Torrent
                        : DownloadType.Mirror;

                string name =
                    type == DownloadType.Torrent
                        ? "Torrent Download"
                        : HtmlEntity.DeEntitize(link.InnerText).Trim();

                if (href.StartsWith("/"))
                    href = "https://sceneryaddons.org" + href;

                methods.Add(new DownloadMethod
                {
                    Name = name,
                    Url = System.Net.WebUtility.HtmlDecode(href),
                    Type = type
                });
            }

            return methods;
        }

        // ================= HELPERS =================
        private string? ExtractIcaoFromTitle(string title)
        {
            var match = Regex.Match(title, @"\b[A-Z]{4}\b");
            return match.Success ? match.Value : null;
        }

        private void NormalizeNameAndDeveloper(Scenario scenario)
        {
            if (string.IsNullOrWhiteSpace(scenario.Name))
                return;

            var parts = scenario.Name.Split(
                new[] { " - ", " – ", " — " },
                2,
                StringSplitOptions.None);

            if (parts.Length < 2)
                return;

            scenario.Developer = parts[0].Trim();
            scenario.Name = parts[1].Trim();
        }
    }
}
