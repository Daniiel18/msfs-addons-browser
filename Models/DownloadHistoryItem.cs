public class DownloadHistoryItem
{
    public string Icao { get; set; } = "";
    public string ScenarioName { get; set; } = "";
    public string Developer { get; set; } = "";
    public string Method { get; set; } = "";
    public DateTime DownloadDate { get; set; }

    public bool IsInstalled { get; set; }
    public bool AutoInstallPending { get; set; }

    public string? PackagePath { get; set; }

    public string ScenarioId => $"{Icao}_{Developer}".ToUpperInvariant();

}
