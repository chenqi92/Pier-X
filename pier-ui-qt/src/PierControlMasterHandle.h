#pragma once

#include <QObject>
#include <QPointer>
#include <QString>
#include <qqml.h>

#include <atomic>
#include <memory>
#include <thread>

struct PierControlMaster;

class PierControlMasterHandle : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierControlMasterHandle)

    Q_PROPERTY(bool connected READ connected NOTIFY connectedChanged FINAL)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)
    Q_PROPERTY(QString target READ target NOTIFY connectedChanged FINAL)

public:
    explicit PierControlMasterHandle(QObject *parent = nullptr);
    ~PierControlMasterHandle() override;

    bool connected() const { return m_connected; }
    bool busy() const { return m_busy; }
    QString target() const { return m_target; }
    ::PierControlMaster *handle() const { return m_handle; }

public slots:
    void connectTo(const QString &host, int port, const QString &user);
    QString exec(const QString &command);
    void close();

signals:
    void connectedChanged();
    void busyChanged();

private slots:
    void onConnectResult(bool ok);

private:
    ::PierControlMaster *m_handle = nullptr;
    bool m_connected = false;
    bool m_busy = false;
    QString m_target;
    std::unique_ptr<std::thread> m_worker;
};
