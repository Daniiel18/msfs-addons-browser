namespace SceneryAddonsBrowser.Models
{
    public enum DownloadState
    {
        Queued,
        WaitingForMagnet,
        ResolvingMagnet,
        Downloading,
        Paused,
        Completed,
        Cancelled,
        Error,
        Installing
    }
}
