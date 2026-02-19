using System.Collections.Generic;
using System.Text.RegularExpressions;

namespace SceneryAddonsBrowser.Services
{
    public static class ChangelogParser
    {
        public static List<string> Parse(string? html)
        {
            var result = new List<string>();

            if (string.IsNullOrWhiteSpace(html))
            {
                result.Add("No changelog provided.");
                return result;
            }

            // 1️⃣ Quitar saltos raros
            html = html.Replace("\r", "").Replace("\n", "");

            // 2️⃣ Extraer <li> si existen
            var liMatches = Regex.Matches(
                html,
                "<li>(.*?)</li>",
                RegexOptions.IgnoreCase);

            if (liMatches.Count > 0)
            {
                foreach (Match match in liMatches)
                {
                    var text = StripTags(match.Groups[1].Value).Trim();
                    if (!string.IsNullOrWhiteSpace(text))
                        result.Add(text);
                }

                return result;
            }

            // 3️⃣ Fallback: <br> o <p>
            html = Regex.Replace(html, "<br ?/?>", "\n", RegexOptions.IgnoreCase);
            html = Regex.Replace(html, "</p>", "\n", RegexOptions.IgnoreCase);

            var lines = StripTags(html)
                .Split('\n', System.StringSplitOptions.RemoveEmptyEntries);

            foreach (var line in lines)
            {
                var clean = line.Trim();
                if (!string.IsNullOrWhiteSpace(clean))
                    result.Add(clean);
            }

            if (result.Count == 0)
                result.Add("No changelog provided.");

            return result;
        }

        private static string StripTags(string input)
        {
            return Regex.Replace(input, "<.*?>", string.Empty);
        }
    }
}
