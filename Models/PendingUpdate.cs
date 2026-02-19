public class PendingUpdate
{
    public string CurrentVersion { get; set; } = "";
    public string NewVersion { get; set; } = "";
    public IEnumerable<string> Changelog { get; set; } = [];
    public Velopack.UpdateInfo UpdateInfo { get; set; } = null!;
}
