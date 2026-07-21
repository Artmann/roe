using Microsoft.AspNetCore.Mvc;

namespace WebApi.Controllers;

public class HealthController : ControllerBase
{
    private readonly IClock clock;

    public HealthController(IClock clock)
    {
        this.clock = clock;
    }

    [HttpGet]
    public string Get()
    {
        return clock.Now().ToString();
    }
}
