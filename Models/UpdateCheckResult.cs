namespace SceneryAddonsBrowser.Services
{
    public class UpdateCheckResult
    {
        public bool IsUpdateAvailable { get; init; }
        public string CurrentVersion { get; init; } = "";
        public string LatestVersion { get; init; } = "";
        public IReadOnlyList<string> Changelog { get; init; } = Array.Empty<string>();

        // Solo se usa si el usuario acepta
        internal Velopack.UpdateInfo? VelopackUpdate { get; init; }
    }
}
