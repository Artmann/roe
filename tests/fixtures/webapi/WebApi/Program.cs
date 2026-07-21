using WebApi;

var builder = WebApplication.CreateBuilder(args);
builder.Services.AddControllers();
builder.Services.AddScoped<IClock, SystemClock>();

var app = builder.Build();
app.MapControllers();
app.Run();
