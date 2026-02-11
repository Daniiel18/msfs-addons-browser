namespace SceneryAddonsBrowser.Models
{
    public enum DownloadState
    {
        Queued,
        ResolvingMagnet,
        Downloading,
        Paused,
        Completed,
        Cancelled,
        Error,
        Installing
    }
}
