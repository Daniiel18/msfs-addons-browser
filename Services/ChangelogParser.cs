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

            // Quitar tags HTML
            var text = Regex.Replace(html, "<.*?>", string.Empty);

            // Normalizar saltos
            var lines = text
                .Replace("\r", "")
                .Split('\n');

            foreach (var line in lines)
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
