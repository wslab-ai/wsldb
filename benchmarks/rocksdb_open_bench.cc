#include <charconv>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <iomanip>
#include <iostream>
#include <limits>
#include <memory>
#include <string>
#include <string_view>
#include <system_error>

#include "rocksdb/cache.h"
#include "rocksdb/db.h"
#include "rocksdb/options.h"
#include "rocksdb/table.h"
#include "rocksdb/version.h"

namespace {

bool ParseSize(std::string_view input, std::size_t* output) {
  std::uint64_t parsed = 0;
  const auto result =
      std::from_chars(input.data(), input.data() + input.size(), parsed);
  if (result.ec != std::errc{} || result.ptr != input.data() + input.size() ||
      parsed == 0 || parsed > std::numeric_limits<std::size_t>::max()) {
    return false;
  }
  *output = static_cast<std::size_t>(parsed);
  return true;
}

std::string EncodeHex(std::string_view input) {
  constexpr char kHex[] = "0123456789abcdef";
  std::string encoded;
  encoded.reserve(input.size() * 2);
  for (const unsigned char byte : input) {
    encoded.push_back(kHex[byte >> 4]);
    encoded.push_back(kHex[byte & 0x0f]);
  }
  return encoded;
}

}  // namespace

int main(int argc, char** argv) {
  if (argc == 2 && std::string_view(argv[1]) == "--version") {
    std::cout << "rocksdb_open_bench version "
              << rocksdb::GetRocksVersionAsString() << '\n';
    return 0;
  }
  if (argc != 4) {
    std::cerr << "usage: rocksdb_open_bench DB KEY_BYTES CACHE_BYTES\n";
    return 2;
  }

  std::size_t key_bytes = 0;
  std::size_t cache_bytes = 0;
  if (!ParseSize(argv[2], &key_bytes) || !ParseSize(argv[3], &cache_bytes) ||
      key_bytes < sizeof(std::uint64_t)) {
    std::cerr << "KEY_BYTES and CACHE_BYTES must be valid positive sizes\n";
    return 2;
  }

  rocksdb::BlockBasedTableOptions table_options;
  table_options.block_cache = rocksdb::NewLRUCache(cache_bytes);
  rocksdb::Options options;
  options.create_if_missing = false;
  options.compression = rocksdb::kNoCompression;
  options.table_factory.reset(rocksdb::NewBlockBasedTableFactory(table_options));

  // db_bench encodes integer zero as eight zero bytes followed by ASCII '0'
  // padding for a 16-byte key. The first eight bytes remain zero for wider keys.
  std::string key(key_bytes, '0');
  key.replace(0, sizeof(std::uint64_t), sizeof(std::uint64_t), '\0');

  const auto started = std::chrono::steady_clock::now();
  std::unique_ptr<rocksdb::DB> database;
  rocksdb::Status status = rocksdb::DB::Open(options, argv[1], &database);
  if (!status.ok()) {
    std::cerr << "DB::Open failed: " << status.ToString() << '\n';
    return 1;
  }
  std::string value;
  status = database->Get(rocksdb::ReadOptions(), key, &value);
  if (!status.ok()) {
    std::cerr << "first Get failed: " << status.ToString() << '\n';
    std::unique_ptr<rocksdb::Iterator> iterator(
        database->NewIterator(rocksdb::ReadOptions()));
    iterator->SeekToFirst();
    if (iterator->Valid()) {
      std::cerr << "expected_key=" << EncodeHex(key)
                << " first_stored_key=" << EncodeHex(iterator->key().ToString())
                << '\n';
    } else if (!iterator->status().ok()) {
      std::cerr << "first-key inspection failed: "
                << iterator->status().ToString() << '\n';
    } else {
      std::cerr << "database contains no keys\n";
    }
    return 1;
  }
  const auto elapsed = std::chrono::steady_clock::now() - started;
  const auto nanos =
      std::chrono::duration_cast<std::chrono::nanoseconds>(elapsed).count();
  const double operations_per_second = 1.0e9 / static_cast<double>(nanos);
  std::cout << "w7_open_to_first_read,0,1," << nanos
            << ',' << std::fixed << std::setprecision(3) << operations_per_second
            << ',' << nanos << ',' << nanos << ',' << nanos << ',' << nanos << '\n';
  return 0;
}
