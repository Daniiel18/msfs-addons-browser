using SceneryAddonsBrowser.Logging;
using SceneryAddonsBrowser.Models;

namespace SceneryAddonsBrowser.Services
{
    public class TorrentQueueService
    {
        private readonly Queue<TorrentJob> _queue = new();
        private bool _isRunning;

        private readonly DownloadService _downloadService;
        public int QueueCount => _queue.Count;

        public event Action? QueueChanged;

        public int TotalCount => _queue.Count + (_isRunning ? 1 : 0);
        public int CurrentIndex => _isRunning ? 1 : 0;


        public TorrentQueueService(DownloadService downloadService)
        {
            _downloadService = downloadService;
        }

        public void Enqueue(TorrentJob job)
        {
            _queue.Enqueue(job);
            QueueChanged?.Invoke();

            AppLogger.Log($"[QUEUE] Added job. Queue size: {_queue.Count}");

            if (!_isRunning)
                _ = ProcessNextAsync();
        }

        private async Task ProcessNextAsync()
        {
            if (_queue.Count == 0)
            {
                _isRunning = false;
                QueueChanged?.Invoke();
                return;
            }

            _isRunning = true;
            QueueChanged?.Invoke();

            var job = _queue.Dequeue();
            await _downloadService.DownloadTorrentInternalAsync(
                job.Method,
                job.Progress
            );

            QueueChanged?.Invoke();
            await ProcessNextAsync();
        }
    }
}
