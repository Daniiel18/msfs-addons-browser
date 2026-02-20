using System.Collections.Generic;
using System.Text.RegularExpressions;

namespace SceneryAddonsBrowser.Update
{
    public static class ChangelogParser
    {
        public static List<string> Parse(string? html)
        {
            var result = new List<string>();

            if (string.IsNullOrWhiteSpace(html))
            {
                result.Add("No release notes were provided.");
                return result;
            }

            // Convertir <li> en líneas
            html = Regex.Replace(html, @"</li>", "\n", RegexOptions.IgnoreCase);
            html = Regex.Replace(html, @"<li[^>]*>", "• ", RegexOptions.IgnoreCase);

            // Quitar el resto de tags
            var text = Regex.Replace(html, "<.*?>", string.Empty);

            foreach (var line in text.Split('\n'))
            {
                var trimmed = line.Trim();
                if (trimmed.Length > 2)
                    result.Add(trimmed);
            }

            if (result.Count == 0)
                result.Add("No release notes were provided.");

            return result;
        }
    }
}
