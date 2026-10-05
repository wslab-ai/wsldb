#include <rocksdb/db.h>
#include <rocksdb/options.h>
#include <rocksdb/statistics.h>

#include <algorithm>
#include <array>
#include <atomic>
#include <barrier>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <iostream>
#include <limits>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

namespace {

using Clock = std::chrono::steady_clock;
constexpr std::size_t kValueBytes = 100;
constexpr std::size_t kWriteBufferBytes = 1024ULL * 1024ULL * 1024ULL;

[[noreturn]] void Fail(const std::string& message) {
  throw std::runtime_error(message);
}

std::uint64_t ParsePositive(const char* text, const char* name) {
  std::size_t parsed = 0;
  const auto value = std::stoull(text, &parsed);
  if (text[parsed] != '\0' || value == 0) {
    Fail(std::string(name) + " must be a positive integer");
  }
  return value;
}

void Require(const rocksdb::Status& status, const char* operation) {
  if (!status.ok()) {
    Fail(std::string(operation) + ": " + status.ToString());
  }
}

std::array<char, 16> Key(std::uint64_t index) {
  std::array<char, 16> key{};
  key.fill('0');
  for (std::size_t offset = 0; offset < 8; ++offset) {
    const auto shift = static_cast<unsigned>((7 - offset) * 8);
    key[offset] = static_cast<char>((index >> shift) & 0xff);
  }
  return key;
}

std::array<char, kValueBytes> Value(std::uint64_t index) {
  std::array<char, kValueBytes> value{};
  std::array<std::uint8_t, 8> seed{};
  for (std::size_t offset = 0; offset < seed.size(); ++offset) {
    seed[offset] = static_cast<std::uint8_t>(index >> (offset * 8));
  }
  for (std::size_t position = 0; position < value.size(); ++position) {
    const auto mask = static_cast<std::uint8_t>(position * 31);
    value[position] = static_cast<char>(seed[position % seed.size()] ^ mask);
  }
  return value;
}

std::uint64_t DirectoryBytes(const std::filesystem::path& directory) {
  std::uint64_t bytes = 0;
  for (const auto& entry : std::filesystem::recursive_directory_iterator(directory)) {
    if (!entry.is_regular_file()) {
      continue;
    }
    const auto size = entry.file_size();
    if (size > std::numeric_limits<std::uint64_t>::max() - bytes) {
      Fail("database size overflow");
    }
    bytes += size;
  }
  return bytes;
}

void Verify(rocksdb::DB* database, std::uint64_t transactions) {
  rocksdb::ReadOptions options;
  std::string found;
  for (std::uint64_t transaction = 0; transaction < transactions; ++transaction) {
    const auto key = Key(transaction);
    const auto expected = Value(transaction);
    found.clear();
    Require(database->Get(options, rocksdb::Slice(key.data(), key.size()), &found),
            "verify get");
    if (found.size() != expected.size() ||
        !std::equal(found.begin(), found.end(), expected.begin())) {
      Fail("verification found an unexpected value");
    }
  }

  std::uint64_t entries = 0;
  std::unique_ptr<rocksdb::Iterator> iterator(database->NewIterator(options));
  for (iterator->SeekToFirst(); iterator->Valid(); iterator->Next()) {
    if (entries == std::numeric_limits<std::uint64_t>::max()) {
      Fail("entry count overflow");
    }
    ++entries;
  }
  Require(iterator->status(), "verify iteration");
  if (entries != transactions) {
    Fail("verification found an unexpected entry count");
  }
}

void Run(const std::filesystem::path& path, std::uint64_t transactions,
         std::uint64_t thread_count) {
  if (std::filesystem::exists(path)) {
    Fail("database path already exists");
  }
  if (thread_count > static_cast<std::uint64_t>(std::numeric_limits<std::ptrdiff_t>::max() - 1)) {
    Fail("thread count is too large");
  }

  auto statistics = rocksdb::CreateDBStatistics();
  rocksdb::Options options;
  options.create_if_missing = true;
  options.compression = rocksdb::kNoCompression;
  options.write_buffer_size = kWriteBufferBytes;
  options.statistics = statistics;
  rocksdb::DB* raw_database = nullptr;
  Require(rocksdb::DB::Open(options, path.string(), &raw_database), "open");
  std::unique_ptr<rocksdb::DB> database(raw_database);

  rocksdb::WriteOptions write_options;
  write_options.sync = true;
  std::atomic<std::uint64_t> next{0};
  std::barrier start(static_cast<std::ptrdiff_t>(thread_count + 1));
  std::mutex failure_mutex;
  std::string failure;
  std::vector<std::thread> workers;
  workers.reserve(static_cast<std::size_t>(thread_count));
  for (std::uint64_t thread = 0; thread < thread_count; ++thread) {
    workers.emplace_back([&] {
      start.arrive_and_wait();
      for (;;) {
        const auto transaction = next.fetch_add(1, std::memory_order_relaxed);
        if (transaction >= transactions) {
          return;
        }
        const auto key = Key(transaction);
        const auto value = Value(transaction);
        const auto status = database->Put(
            write_options, rocksdb::Slice(key.data(), key.size()),
            rocksdb::Slice(value.data(), value.size()));
        if (!status.ok()) {
          std::lock_guard lock(failure_mutex);
          if (failure.empty()) {
            failure = status.ToString();
          }
          return;
        }
      }
    });
  }

  const auto started = Clock::now();
  start.arrive_and_wait();
  for (auto& worker : workers) {
    worker.join();
  }
  const auto elapsed = Clock::now() - started;
  if (!failure.empty()) {
    Fail("concurrent put: " + failure);
  }

  const auto syncs = statistics->getTickerCount(rocksdb::WAL_FILE_SYNCED);
  const auto wal_bytes = statistics->getTickerCount(rocksdb::WAL_FILE_BYTES);
  rocksdb::HistogramData sync{};
  statistics->histogramData(rocksdb::WAL_FILE_SYNC_MICROS, &sync);
  Verify(database.get(), transactions);
  database.reset();
  const auto elapsed_ns =
      std::chrono::duration_cast<std::chrono::nanoseconds>(elapsed).count();
  const auto throughput = static_cast<double>(transactions) * 1.0e9 /
                          static_cast<double>(elapsed_ns);
  const auto storage_bytes = DirectoryBytes(path);

  std::cout << "mode,transactions,threads,elapsed_ns,transactions_per_second,"
               "wal_syncs,wal_bytes,storage_bytes,verified_entries\n";
  std::cout << "automatic," << transactions << ',' << thread_count << ','
            << elapsed_ns << ',' << throughput << ',' << syncs << ','
            << wal_bytes << ',' << storage_bytes << ',' << transactions << '\n';
  std::cerr << "rocksdb_wal_stats syncs=" << syncs
            << " histogram_count=" << sync.count
            << " sync_average_micros=" << sync.average
            << " sync_p99_micros=" << sync.percentile99
            << " sync_max_micros=" << sync.max << '\n';
}

}  // namespace

int main(int argc, char** argv) {
  try {
    if (argc != 4) {
      Fail("usage: rocksdb_group_commit_bench DB TRANSACTIONS THREADS");
    }
    Run(argv[1], ParsePositive(argv[2], "TRANSACTIONS"),
        ParsePositive(argv[3], "THREADS"));
    return 0;
  } catch (const std::exception& error) {
    std::cerr << error.what() << '\n';
    return 1;
  }
}
