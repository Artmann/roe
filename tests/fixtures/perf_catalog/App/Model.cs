using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace App;

public class Order
{
    public int CustomerId { get; set; }

    public string Sku { get; set; } = "";

    public DateTime ProcessedAt { get; set; }
}

public class Customer
{
    public int Id { get; set; }
}

public class AppDatabase
{
    public List<Customer> Customers { get; set; } = new List<Customer>();

    public List<string> Settings { get; set; } = new List<string>();

    public Task SaveChangesAsync()
    {
        return Task.CompletedTask;
    }
}
