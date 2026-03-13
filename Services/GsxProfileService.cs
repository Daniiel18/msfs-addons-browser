using Microsoft.Playwright;
using SceneryAddonsBrowser.Models;
using System.Net.Http;
using System.Text.RegularExpressions;

public class GsxProfileService
{
    private readonly HttpClient _httpClient = new();

    public async Task CheckGsxProfileAsync(Scenario scenario)
    {
        if (scenario == null || string.IsNullOrWhiteSpace(scenario.Icao))
            return;

        var icao = scenario.Icao.ToLowerInvariant();
        var url = $"https://flightsim.to/miscellaneous/gsx-pro?q={icao}";

        try
        {
            var html = await _httpClient.GetStringAsync(url);

            var links = Regex.Matches(
                html,
                @"href=""(\/addon\/[^""]+)""",
                RegexOptions.IgnoreCase
            )
            .Cast<Match>()
            .Select(m => m.Groups[1].Value)
            .Distinct()
            .ToList();

            int count = links.Count;

            if (count > 0)
            {
                scenario.GsxText =
                    $"GSX Profiles available ({count}) — View on flightsim.to";

                scenario.GsxUrl = url;
            }
            else
            {
                scenario.GsxText =
                    "GSX Profiles — Not found";
            }
        }
        catch
        {
            scenario.GsxText =
                "GSX Profiles — Not available";
        }
    }

    public async Task<int> GetProfileCountAsync(string icao)
    {
        if (string.IsNullOrWhiteSpace(icao))
            return 0;

        var url = $"https://flightsim.to/miscellaneous/gsx-pro?q={icao.ToLower()}";

        using var playwright = await Playwright.CreateAsync();

        await using var browser = await playwright.Chromium.LaunchAsync(
            new BrowserTypeLaunchOptions
            {
                Headless = true
            });

        var page = await browser.NewPageAsync();

        await page.GotoAsync(url);

        // Esperar que carguen los perfiles
        await page.WaitForSelectorAsync("a[href^='/addon/']", new()
        {
            Timeout = 10000
        });

        var links = await page.QuerySelectorAllAsync("a[href^='/addon/']");

        return links.Count;
    }
}