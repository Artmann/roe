using static Kill.MathHelper;

namespace Kill;

public static class Program
{
    public static void Main()
    {
        IGreeter greeter = new PoliteGreeter();
        greeter.Greet();

        Shape shape = new Circle();
        shape.Render();

        var services = new ServiceCollection();
        services.AddScoped<IRepo, SqlRepo>();
        services.AddHostedService<Worker>();

        IRepo repo = new SqlRepo();
        repo.Save();

        var dto = JsonSerializer.Deserialize<UserDto>("{}");
        var shouted = "hello".Shout();
        var name = nameof(Order.Total);
        var migration = typeof(Migration001);

        var button = new Button();
        button.Click += OnClicked;

        var squared = Square(2);

        var live = new LiveService();
        live.Process();

        var host = new ClusterHost();
        var status = Status.Active;
    }

    private static void OnClicked(object sender, EventArgs e)
    {
    }
}
