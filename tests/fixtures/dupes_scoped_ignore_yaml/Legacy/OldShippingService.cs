using System;
using System.Threading.Tasks;

namespace Billing.Legacy;

// Matched only by the `dupes.ignore` glob: its retry-method occurrence must
// vanish from the duplicate report, while this file's dead-code finding and
// the health finding for Route stay.
public class OldShippingService
{
    public Task DispatchAsync(string orderId)
    {
        return ExecuteWithRetryAsync(async () =>
        {
            Console.WriteLine($"Dispatching order {orderId}");
            await Task.CompletedTask;
        }, maxAttempts: 5);
    }

    public string Route(bool express, bool oversized, bool fragile)
    {
        if (express)
        {
            return "air";
        }
        else if (oversized && fragile)
        {
            return "special";
        }

        return "ground";
    }

    private async Task ExecuteWithRetryAsync(Func<Task> action, int maxAttempts)
    {
        var attempt = 0;

        while (true)
        {
            try
            {
                await action();
                return;
            }
            catch (Exception exception) when (attempt < maxAttempts)
            {
                attempt++;
                var delay = TimeSpan.FromSeconds(Math.Pow(2, attempt));
                Console.WriteLine($"Retry {attempt} after {exception.Message}, waiting {delay}");
                await Task.Delay(delay);
            }
        }
    }
}

internal class LegacyLedger
{
    public void DoNothing()
    {
    }
}
