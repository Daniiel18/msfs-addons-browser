using HtmlAgilityPack;
using HtmlDocument = HtmlAgilityPack.HtmlDocument;

namespace SceneryAddonsBrowser.Update
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

            var doc = new HtmlDocument();
            doc.LoadHtml(html);

            var listItems = doc.DocumentNode.SelectNodes("//li");

            if (listItems != null && listItems.Count > 0)
            {
                foreach (var li in listItems)
                {
                    var text = li.InnerText.Trim();
                    if (!string.IsNullOrWhiteSpace(text))
                        result.Add(text);
                }

                return result;
            }

            var plainText = HtmlEntity.DeEntitize(doc.DocumentNode.InnerText)
                .Trim();

            if (!string.IsNullOrWhiteSpace(plainText))
                result.Add(plainText);

            return result;
        }
    }
}
