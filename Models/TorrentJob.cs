namespace SceneryAddonsBrowser.Models
{
    public class TorrentJob
    {
        public DownloadMethod Method { get; }
        public Action<string, int>? Progress { get; }

        public TorrentJob(
            DownloadMethod method,
            Action<string, int>? progress)
        {
            Method = method;
            Progress = progress;
        }
    }
}
