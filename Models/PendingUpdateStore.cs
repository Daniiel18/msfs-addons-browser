using SceneryAddonsBrowser.Models;

namespace SceneryAddonsBrowser.Update
{
    public static class PendingUpdateStore
    {
        public static PendingUpdate? PendingUpdate { get; set; }

        public static void Clear()
        {
            PendingUpdate = null;
        }
    }
}
