using HtmlAgilityPack;
using SceneryAddonsBrowser.Logging;
using SceneryAddonsBrowser.Models;
using System.Diagnostics;
using System.Net.Http;
using HtmlDocument = HtmlAgilityPack.HtmlDocument;

public class GsxProfileService
{
    private readonly HttpClient _httpClient = new();

    public async Task CheckGsxProfileAsync(Scenario scenario)
    {
        if (scenario == null || string.IsNullOrWhiteSpace(scenario.Icao))
            return;

        var icao = scenario.Icao.ToLowerInvariant();
        var url = $"https://flightsim.to/others/gsx-pro/search/{icao}";

        try
        {
            var html = await _httpClient.GetStringAsync(url);

            if (html.Contains("/file/", StringComparison.OrdinalIgnoreCase))
            {
                scenario.GsxText = "GSX Profiles — View available profiles";
                scenario.GsxUrl = url;
            }
            else
            {
                scenario.GsxText = "GSX Profiles — Not found";
            }
        }
        catch
        {
            scenario.GsxText = "GSX Profiles — Not available";
        }
    }
}