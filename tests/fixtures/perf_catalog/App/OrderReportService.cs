using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace App;

/// Every check that needs a loop around it to fire.
public class OrderReportService
{
    private readonly AppDatabase _database;

    /// A linear scan the catalog can see through the declaration.
    private readonly List<string> _blockedSkus = new List<string>();

    /// The same call on a hashed collection, which must stay silent.
    private readonly HashSet<string> _allowedSkus = new HashSet<string>();

    public OrderReportService(AppDatabase database)
    {
        _database = database;
    }

    public async Task<string> BuildAsync(List<Order> orders)
    {
        string report = "";

        foreach (var order in orders)
        {
            var customer = await _database.Customers.FirstOrDefaultAsync(c => c.Id == order.CustomerId);
            var settings = await _database.Settings.FirstOrDefaultAsync();

            if (_blockedSkus.Contains(order.Sku))
            {
                continue;
            }

            if (_allowedSkus.Contains(order.Sku))
            {
                report += order.Sku;
            }

            order.ProcessedAt = DateTime.Now;

            await _database.SaveChangesAsync();
            await Task.Delay(1);
        }

        return report;
    }
}
