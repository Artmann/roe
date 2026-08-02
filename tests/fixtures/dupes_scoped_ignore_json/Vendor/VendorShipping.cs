using System;
using System.Threading.Tasks;

namespace Billing.Vendor;

// Matched by the top-level `ignore` glob, which applies to every command:
// neither the duplicate occurrence nor this file's dead-code finding may
// appear anywhere.
public class VendorShipping
{
    public Task ShipAsync(string parcelId)
    {
        return ExecuteWithRetryAsync(async () =>
        {
            Console.WriteLine($"Shipping parcel {parcelId}");
            await Task.CompletedTask;
        }, maxAttempts: 4);
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

internal class VendorLedger
{
    public void DoNothing()
    {
    }
}
