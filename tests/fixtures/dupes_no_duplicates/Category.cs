using System.Collections.Generic;

namespace Catalog;

public class Category
{
    public int Id { get; set; }

    public string Title { get; set; } = string.Empty;

    public List<Product> Products { get; set; } = new();

    public int CountInStock()
    {
        var count = 0;
        foreach (var product in Products)
        {
            if (product.Price > 0)
            {
                count++;
            }
        }
        return count;
    }
}
