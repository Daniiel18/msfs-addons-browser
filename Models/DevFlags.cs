namespace SceneryAddonsBrowser
{
    public static class DevFlags
    {
#if DEBUG
        public const bool ForceUpdateDialog = true;
#else
        public const bool ForceUpdateDialog = false;
#endif
    }
}
