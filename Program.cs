using Velopack;

namespace SceneryAddonsBrowser
{
    public static class Program
    {
        [STAThread]
        public static void Main(string[] args)
        {
            // NECESARIO PARA VELOPACK
            VelopackApp.Build().Run();

            var app = new App();
            app.InitializeComponent();
            app.Run();
        }
    }
}
